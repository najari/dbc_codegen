"""Compare compiled generated payloads, independent literals and a pinned candb DLL."""
import argparse
import ctypes as c
import hashlib
import json
import math
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dll", required=True, type=Path)
    parser.add_argument("--exe", type=Path, default=ROOT / "target/debug/dbc-codegen.exe")
    parser.add_argument("--report", type=Path, default=ROOT / "artifacts/dll-report.json")
    args = parser.parse_args()
    source = ROOT / "tests/fixtures/simulation.dbc"
    source_bytes = source.read_bytes()
    digest = lambda value: hashlib.sha256(value).hexdigest()
    dll_hash = digest(args.dll.read_bytes())
    api = c.CDLL(str(args.dll.resolve()))
    api.candb_new.restype = c.c_void_p
    api.candb_free.argtypes = [c.c_void_p]
    api.candb_load_text.argtypes = [c.c_void_p, c.c_void_p, c.c_size_t]
    api.candb_load_text.restype = c.c_int
    callback_type = c.CFUNCTYPE(None, c.c_void_p, c.POINTER(Signal))
    api.candb_decode_ex.argtypes = [c.c_void_p, c.c_uint32, c.c_void_p, c.c_size_t, callback_type, c.c_void_p]
    api.candb_decode_ex.restype = c.c_int
    api.candb_last_error.argtypes = [c.c_void_p]
    api.candb_last_error.restype = c.c_char_p
    expected = [
        (105, [0, 0, 192, 191], {"Value": (0xbfc00000, -1.5)}),
        (106, [63, 192, 0, 0], {"Value": (0x3fc00000, 1.5)}),
        (105, [52, 18, 192, 127], {"Value": (0x7fc01234, None)}),
        (107, [0, 0, 0, 0, 0, 0, 0, 128], {"Value": (0x8000000000000000, -0.0)}),
        (108, [191, 248, 0, 0, 0, 0, 0, 0], {"Value": (0xbff8000000000000, -1.5)}),
        (109, [255] * 8, {"Value": (2**64 - 1, float(2**64 - 1))}),
        (110, [128, 0, 0, 0, 0, 0, 0, 0], {"Value": (-2**63, float(-2**63))}),
        (113, [255], {"Value": (-1, -10.5)}),
        (114, [65], {"Value": (65, -120.0)}),
        (115, [1, 160, 85], {"Switch": (1, 1.0), "Always": (85, 85.0), "A": (0, 0.0)}),
        (115, [2, 48, 85], {"Switch": (2, 2.0), "Always": (85, 85.0), "B": (3, 3.0)}),
        (116, [255], {"Mode": (-1, -12.0)}),
    ]
    with tempfile.TemporaryDirectory(prefix="dbc-dll-") as temporary:
        temp = Path(temporary)
        run([str(args.exe.resolve()), str(source), str(temp)])
        generator_manifest = json.loads((temp / "manifest.json").read_text(encoding="utf8"))
        program = r'''
mod messages; use messages::*;
fn emit(id: u32, bytes: &[u8]) { println!("{{\"id\":{},\"payload\":{:?}}}", id, bytes); }
fn main() {
    emit(105, FloatLe::new(-1.5).unwrap().raw());
    emit(106, FloatBe::new(1.5).unwrap().raw());
    let mut f = FloatLe::new(0.0).unwrap(); f.set_value_raw_val(0x7fc01234).unwrap(); emit(105, f.raw());
    emit(107, DoubleLe::new(-0.0).unwrap().raw());
    emit(108, DoubleBe::new(-1.5).unwrap().raw());
    emit(109, Wide::new(u64::MAX).unwrap().raw());
    emit(110, SignedWide::new(i64::MIN).unwrap().raw());
    emit(113, Scaled::new(-10.5).unwrap().raw());
    emit(114, Negative::new(-120).unwrap().raw());
    let mut mux = Mux::try_from(&[1, 0xa5, 0x55][..]).unwrap();
    let mut a = MuxSwitchM1::new(); a.set_a(0).unwrap(); mux.set_m1(a).unwrap(); emit(115, mux.raw());
    let mut b = MuxSwitchM2::new(); b.set_b(3).unwrap(); mux.set_m2(b).unwrap(); emit(115, mux.raw());
    emit(116, Choice::new(ChoiceMode::Negative).unwrap().raw());
}
'''
        main_rs = temp / "main.rs"
        main_rs.write_text(program, encoding="utf8")
        deps = ROOT / "target/debug/deps"
        command = ["rustc", "--edition=2024", str(main_rs), "-L", f"dependency={deps}"]
        for name in ["bitvec", "embedded_can"]:
            command += ["--extern", f"{name}={next(deps.glob(f'lib{name}-*.rlib'))}"]
        executable = temp / "vectors.exe"
        run([*command, "-o", str(executable)])
        frames = [json.loads(row) for row in run([str(executable)]).splitlines()]
        assert len(frames) == len(expected)
        handle = api.candb_new()
        if not handle:
            raise RuntimeError("candb_new failed")
        checks = []
        try:
            data = c.create_string_buffer(source_bytes)
            assert api.candb_load_text(handle, data, len(source_bytes)) == 0, api.candb_last_error(handle)
            for frame, (id_value, literal, signals) in zip(frames, expected):
                assert frame["id"] == id_value and frame["payload"] == literal, (frame, literal)
                observed = {}

                @callback_type
                def receive(_, pointer):
                    s = pointer.contents
                    observed[s.name.decode()] = {"raw": s.raw_signed if s.is_signed else s.raw_unsigned,
                        "physical": s.physical, "status": s.status}

                payload = (c.c_uint8 * len(literal))(*literal)
                count = api.candb_decode_ex(handle, id_value, payload, len(literal), receive, None)
                assert count == len(signals), (id_value, count, observed)
                assert set(observed) == set(signals), (id_value, observed)
                for name, (raw, physical) in signals.items():
                    actual = observed[name]
                    assert actual["status"] == 0
                    # IEEE declarations may preserve a DBC '-' flag; compare exact unsigned bits.
                    if id_value in (105, 106, 107, 108):
                        assert actual["raw"] & ((1 << 64) - 1) == raw
                    else:
                        assert actual["raw"] == raw, (id_value, name, actual, raw)
                    if physical is None:
                        assert math.isnan(actual["physical"])
                        actual["physical"] = "NaN"
                    else:
                        assert actual["physical"] == physical
                        if physical == 0:
                            actual["signed_zero_sign_matches"] = math.copysign(1, actual["physical"]) == math.copysign(1, physical)
                checks.append({**frame, "signals": observed, "passed": True})
        finally:
            api.candb_free(handle)
    assert source.read_bytes() == source_bytes
    report = {"dll": str(args.dll.resolve()), "dll_sha256": dll_hash,
              "generator_manifest": generator_manifest, "rustc": run(["rustc", "--version"]).strip(),
              "source_sha256": digest(source_bytes), "source_unchanged": True,
              "comparison_limit": "DLL physical scaling may canonicalize negative zero; raw bits are compared exactly and generator signed-zero preservation is tested independently",
              "vectors_passed": len(checks), "checks": checks}
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2, allow_nan=False) + "\n", encoding="utf8")
    print(f"{len(checks)} compiled payload / literal byte / DLL vectors passed: {args.report}")


if __name__ == "__main__":
    main()
