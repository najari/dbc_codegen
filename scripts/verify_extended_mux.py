"""Compile corpus mux codecs; compare active wire values and masked writes with DLL."""
import argparse
import ctypes as c
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


class Signal(c.Structure):
    _fields_ = [("name", c.c_char_p), ("unit", c.c_char_p), ("label", c.c_char_p),
                ("physical", c.c_double), ("raw_signed", c.c_int64),
                ("raw_unsigned", c.c_uint64), ("is_signed", c.c_int), ("status", c.c_int)]


def run(command):
    result = subprocess.run(command, capture_output=True, text=True, encoding="utf8")
    if result.returncode:
        raise RuntimeError(result.stderr)
    return result.stdout


def positions(signal):
    start, width = signal["start_bit"], signal["bit_width"]
    if signal["byte_order"] == "LittleEndian":
        return [(start + i, i) for i in range(width)]
    linear = start // 8 * 8 + 7 - start % 8
    return [((linear + i) // 8 * 8 + 7 - (linear + i) % 8, width - 1 - i)
            for i in range(width)]


def put(payload, signal, value):
    for bit, shift in positions(signal):
        mask = 1 << (bit % 8)
        payload[bit // 8] = ((payload[bit // 8] & ~mask)
                            | (((value >> shift) & 1) << (bit % 8)))


def get(payload, signal):
    return sum(((payload[bit // 8] >> (bit % 8)) & 1) << shift
               for bit, shift in positions(signal))


def frames(message):
    signals = message["signals"]
    by_name = {s["source_name"]: s for s in signals}
    initial = [0xa5] * message["size"]
    candidates = [initial]

    def activate(payload, signal):
        parent = signal["mux"]["parent"]
        if parent:
            switch = by_name[parent["source_name"]]
            activate(payload, switch)
            put(payload, switch, parent["ranges"][0]["min"])

    for selector in (s for s in signals if s["mux"]["selector"]):
        mask = (1 << selector["bit_width"]) - 1
        keys = {0, mask}
        for child in signals:
            dep = child["mux"]["parent"]
            if dep and dep["source_name"] == selector["source_name"]:
                for interval in dep["ranges"]:
                    lo, hi = interval["min"], interval["max"]
                    keys.update((lo, hi))
                    if lo > 0:
                        keys.add(lo - 1)
                    if hi < mask:
                        keys.add(hi + 1)
        for key in sorted(keys):
            payload = initial.copy()
            activate(payload, selector)
            put(payload, selector, key)
            candidates.append(payload)
    return [list(p) for p in dict.fromkeys(tuple(p) for p in candidates)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dll", required=True, type=Path)
    parser.add_argument("--exe", type=Path, default=ROOT / "target/debug/dbc-codegen.exe")
    parser.add_argument("--report", type=Path, default=ROOT / "artifacts/extended-mux-report.json")
    args = parser.parse_args()
    api = c.CDLL(str(args.dll.resolve()))
    api.candb_new.restype = c.c_void_p
    api.candb_free.argtypes = [c.c_void_p]
    api.candb_load_text.argtypes = [c.c_void_p, c.c_void_p, c.c_size_t]
    api.candb_load_text.restype = c.c_int
    callback = c.CFUNCTYPE(None, c.c_void_p, c.POINTER(Signal))
    api.candb_decode_ex.argtypes = [c.c_void_p, c.c_uint32, c.c_void_p,
                                  c.c_size_t, callback, c.c_void_p]
    api.candb_decode_ex.restype = c.c_int
    api.candb_last_error.argtypes = [c.c_void_p]
    api.candb_last_error.restype = c.c_char_p
    folder = ROOT / "dbc_samples/cantools/dbc"
    names = ["bus_comment", "multiplex", "multiplex_dumped", "multiplex_choices",
             "multiplex_choices_dumped", "multiplex_2", "multiplex_2_dumped"]
    names += [f"issue_184_extended_mux_{kind}{suffix}"
              for kind in ("cascaded", "independent_multiplexors", "multiple_values")
              for suffix in ("", "_dumped")]
    deps = ROOT / "target/debug/deps"
    externs = []
    for library in ("bitvec", "embedded_can"):
        rlib = max(deps.glob(f"lib{library}-*.rlib"), key=lambda p: p.stat().st_mtime)
        externs += ["--extern", f"{library}={rlib}"]
    reports = []
    total_frames = total_writes = 0
    for name in names:
        source = folder / f"{name}.dbc"
        data = source.read_bytes()
        handle = api.candb_new()
        if not handle:
            raise RuntimeError("candb_new failed")
        try:
            buffer = c.create_string_buffer(data)
            assert api.candb_load_text(handle, buffer, len(data)) == 0, api.candb_last_error(handle)
            with tempfile.TemporaryDirectory(prefix="dbc-extended-mux-") as directory:
                temp = Path(directory)
                run([str(args.exe.resolve()), str(source), str(temp)])
                manifest = json.loads((temp / "manifest.json").read_text(encoding="utf8"))
                checks = []
                statements = ["mod messages; use messages::*; fn main() {"]
                for message in manifest["messages"]:
                    if message["mux_api"] != "guarded":
                        continue
                    for payload in frames(message):
                        index = len(checks)
                        checks.append((message, payload))
                        ty = message["type_name"]
                        # Explicit slice type avoids default i32 array literals.
                        statements.append(f"let raw: &[u8] = &{payload}; let x = {ty}::try_from(raw).unwrap();")
                        for signal in message["signals"]:
                            field, label = signal["field_name"], signal["source_name"]
                            mask = (1 << signal["bit_width"]) - 1
                            statements.append(f'println!("R|{index}|{label}|{{}}|{{}}", x.{field}_is_active(), (x.{field}_raw_val() as u64) & {mask}_u64);')
                            statements.append(f"let mut y = {ty}::try_from(raw).unwrap();")
                            if signal["mux"]["selector"]:
                                value = 0 if get(payload, signal) else 1
                                call = f"y.select_{field}({value})"
                            else:
                                value = 0
                                call = f"y.set_{field}_raw_val(0)"
                            statements.append(f'println!("W|{index}|{label}|{{}}|{{:?}}", {call}.is_ok(), y.raw());')
                statements.append("}")
                assert checks, f"no guarded mux messages in {source.name}"
                (temp / "main.rs").write_text("\n".join(statements), encoding="utf8")
                binary = temp / "vectors.exe"
                run(["rustc", "--edition=2024", str(temp / "main.rs"), "-L",
                     f"dependency={deps}", *externs, "-o", str(binary)])
                reads, writes = {}, {}
                for row in run([str(binary)]).splitlines():
                    kind, index, label, active, value = row.split("|", 4)
                    entry = (active == "true", json.loads(value))
                    (reads if kind == "R" else writes).setdefault(int(index), {})[label] = entry
                receipts = []
                for index, (message, payload) in enumerate(checks):
                    observed = {}

                    @callback
                    def receive(_, pointer):
                        signal = pointer.contents
                        observed[signal.name.decode()] = (signal.raw_unsigned, signal.status)

                    wire_id = message["id"] | (0x80000000 if message["extended"] else 0)
                    raw = (c.c_uint8 * len(payload))(*payload)
                    count = api.candb_decode_ex(handle, wire_id, raw, len(payload), receive, None)
                    assert count >= 0, api.candb_last_error(handle)
                    active_names = {label for label, (active, _) in reads[index].items() if active}
                    assert active_names == set(observed), (source.name, message["source_name"], payload, active_names, observed)
                    for signal in message["signals"]:
                        label = signal["source_name"]
                        active, value = reads[index][label]
                        mask = (1 << signal["bit_width"]) - 1
                        assert value == get(payload, signal), (label, value, payload)
                        if active:
                            dll_raw, status = observed[label]
                            assert status == 0 and value == dll_raw & mask, (label, value, dll_raw, status)
                        ok, actual = writes[index][label]
                        assert ok == active, (label, active, ok)
                        expected = payload.copy()
                        if active:
                            value = (0 if get(payload, signal) else 1) if signal["mux"]["selector"] else 0
                            put(expected, signal, value)
                        assert actual == expected, (source.name, label, actual, expected)
                        total_writes += 1
                    receipts.append({"message": message["source_name"], "payload": payload,
                                     "active_signals": sorted(active_names),
                                     "wire_values": {s["source_name"]: reads[index][s["source_name"]][1]
                                                     for s in message["signals"] if s["source_name"] in active_names},
                                     "passed": True})
                total_frames += len(checks)
                assert source.read_bytes() == data
                reports.append({"sample": str(source.relative_to(ROOT)),
                                "input_sha256": hashlib.sha256(data).hexdigest(),
                                "generator_source_sha256": manifest["generator_source_sha256"],
                                "frames_passed": len(checks), "checks": receipts,
                                "source_unchanged": True})
                print(f"{source.name}: {len(checks)} DLL frame comparisons passed")
        finally:
            api.candb_free(handle)
    report = {"dll_sha256": hashlib.sha256(args.dll.read_bytes()).hexdigest(),
              "rustc": run(["rustc", "--version"]).strip(), "samples_passed": len(reports),
              "frames_passed": total_frames, "masked_writes_passed": total_writes,
              "comparison": "active signal sets, exact wire bits, masked writes and inactive-write preservation",
              "samples": reports}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n", encoding="utf8")
    print(f"{len(reports)} samples, {total_frames} frames, {total_writes} masked writes passed")


if __name__ == "__main__":
    main()
