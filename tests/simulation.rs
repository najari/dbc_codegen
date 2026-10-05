use dbc_codegen::{Config, FeatureConfig, InputEncoding, RoundingPolicy, decode_input};
use std::{fs, path::PathBuf, process::Command};

const DBC: &str = include_str!("fixtures/simulation.dbc");

fn config(content: &str) -> Config<'_> {
    Config::builder()
        .dbc_name("합성 시험.dbc")
        .dbc_content(content)
        .impl_debug(FeatureConfig::Always)
        .allow_dead_code(true)
        .build()
}

fn compile_and_run(code: &str, assertions: &str) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("messages.rs"), code).unwrap();
    fs::write(
        dir.path().join("main.rs"),
        format!("mod messages; use messages::*; fn main() {{ {assertions} }}"),
    )
    .unwrap();
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let rlib = |name: &str| -> PathBuf {
        fs::read_dir(&deps)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                let f = p.file_name().unwrap().to_string_lossy();
                f.starts_with(&format!("lib{name}-")) && f.ends_with(".rlib")
            })
            .unwrap_or_else(|| panic!("missing rlib for {name}"))
    };
    let exe = dir
        .path()
        .join(if cfg!(windows) { "check.exe" } else { "check" });
    let result = Command::new("rustc")
        .arg("--edition=2024")
        .arg(dir.path().join("main.rs"))
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("bitvec={}", rlib("bitvec").display()))
        .arg("--extern")
        .arg(format!("embedded_can={}", rlib("embedded_can").display()))
        .arg("--extern")
        .arg(format!("arbitrary={}", rlib("arbitrary").display()))
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new(exe).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    fs::write(
        dir.path().join("embedded.rs"),
        "#![no_std]\nmod messages;\n",
    )
    .unwrap();
    let result = Command::new("rustc")
        .args(["--edition=2024", "--crate-type=lib", "--emit=metadata"])
        .arg(dir.path().join("embedded.rs"))
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("bitvec={}", rlib("bitvec").display()))
        .arg("--extern")
        .arg(format!("embedded_can={}", rlib("embedded_can").display()))
        .arg("--extern")
        .arg(format!("arbitrary={}", rlib("arbitrary").display()))
        .arg("-o")
        .arg(dir.path().join("embedded.rmeta"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "no_std compilation: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn signed_and_unsigned_all_widths_both_byte_orders() {
    let mut input = "VERSION \"\"\nNS_ :\nBS_:\nBU_: ECU\n".to_owned();
    let mut assertions = String::new();
    let mut id = 1;
    for width in 1..=64 {
        for signed in [false, true] {
            for big in [false, true] {
                let name = format!(
                    "Width{width}{}{}",
                    if signed { "Signed" } else { "Unsigned" },
                    if big { "Be" } else { "Le" }
                );
                let (lo, hi) = if signed {
                    (-(1i128 << (width - 1)), (1i128 << (width - 1)) - 1)
                } else {
                    (0, (1i128 << width) - 1)
                };
                input += &format!(
                    "BO_ {id} {name}: 8 ECU\n SG_ Value : {}|{width}@{}{} (1,0) [{lo}|{hi}] \"\" ECU\n",
                    if big { 7 } else { 0 },
                    u8::from(!big),
                    if signed { '-' } else { '+' }
                );
                let value = if signed { lo } else { hi };
                let wire = (value as u128) & ((1u128 << width) - 1);
                let mut bytes = [0u8; 8];
                for bit in 0..width {
                    let source_bit = if big { width - bit - 1 } else { bit };
                    if wire & (1u128 << source_bit) != 0 {
                        let byte = bit / 8;
                        let offset = if big { 7 - bit % 8 } else { bit % 8 };
                        bytes[byte] |= 1 << offset;
                    }
                }
                let argument = if width == 1 && !signed {
                    "true".to_owned()
                } else {
                    value.to_string()
                };
                assertions += &format!(
                    "let m = {name}::new({argument}).unwrap(); assert_eq!(*m.raw(), {bytes:?}); assert_eq!(m.value_raw_val() as i128, {value}_i128);\n"
                );
                id += 1;
            }
        }
    }
    compile_and_run(&config(&input).generate().unwrap(), &assertions);
}

#[test]
fn generated_code_compiles_and_matches_literal_vectors() {
    compile_and_run(
        &config(DBC).generate().unwrap(),
        include_str!("simulation_runtime.rs.txt"),
    );
}

#[test]
fn arbitrary_with_ieee_f64_and_raw_enums_compiles() {
    let cfg = Config::builder()
        .dbc_name("arb")
        .dbc_content(DBC)
        .physical_f64(true)
        .impl_debug(FeatureConfig::Always)
        .impl_arbitrary(FeatureConfig::Always)
        .allow_dead_code(true)
        .build();
    compile_and_run(
        &cfg.generate().unwrap(),
        "use arbitrary::Arbitrary; let mut u = arbitrary::Unstructured::new(&[0; 1024]); let _ = FloatLe::arbitrary(&mut u); let _ = Choice::arbitrary(&mut u);",
    );
}

#[test]
fn f64_rounding_overflow_and_quantization() {
    let dbc = "VERSION \"\"\nNS_ :\nBS_:\nBU_: ECU\nBO_ 1 Quantized: 1 ECU\n SG_ Value : 0|8@1- (0.5,-10) [-74|53.5] \"\" ECU\n";
    for (policy, expected) in [
        (RoundingPolicy::Truncate, "-9.5"),
        (RoundingPolicy::NearestAway, "-9.0"),
    ] {
        let code = Config::builder()
            .dbc_name("q")
            .dbc_content(dbc)
            .physical_f64(true)
            .rounding(policy)
            .impl_debug(FeatureConfig::Always)
            .build()
            .generate()
            .unwrap();
        compile_and_run(
            &code,
            &format!(
                "let mut q = Quantized::new(0.0).unwrap(); assert_eq!(q.set_value_quantized(-9.25).unwrap(), {expected}); let bytes = *q.raw(); assert!(q.set_value(f64::INFINITY).is_err()); assert!(q.set_value(1000.0).is_err()); assert_eq!(*q.raw(), bytes);"
            ),
        );
    }
    let code = Config::builder()
        .dbc_name("q")
        .dbc_content(dbc)
        .physical_f64(true)
        .rounding(RoundingPolicy::Exact)
        .impl_debug(FeatureConfig::Always)
        .build()
        .generate()
        .unwrap();
    compile_and_run(
        &code,
        "let mut q = Quantized::new(0.0).unwrap(); let bytes = *q.raw(); assert!(q.set_value(-9.25).is_err()); assert_eq!(*q.raw(), bytes);",
    );
}

#[test]
fn invalid_definitions_fail_without_changing_output() {
    let good = "VERSION \"\"\nNS_ :\nBS_:\nBU_: ECU\nBO_ 100 Bad: 1 ECU\n SG_ Value : 0|8@1+ (1,0) [0|255] \"\" ECU\n";
    let invalid = [
        good.replace("BO_ 100", "BO_ 65536"),
        good.replace("BO_ 100", "BO_ 3758096484"),
        good.replace("0|8", "0|0"),
        good.replace("0|8", "7|8"),
        good.replace("(1,0)", "(0,0)"),
        good.replace("[0|255]", "[255|0]"),
        good.replace(": 1 ECU", ": 9 ECU"),
        format!("{good}SIG_VALTYPE_ 100 Value : 1;\n"),
        good.replace("Value :", "Value m1 :"),
        format!("{good}SG_MUL_VAL_ 100 Value Missing 1-2;\n"),
        format!("{good}BO_ 100 Duplicate: 0 ECU\n"),
        format!("{good}BO_TX_BU_ 101 : ECU;\n"),
    ];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("messages.rs");
    for input in invalid {
        fs::write(&path, "previous valid output").unwrap();
        let result = config(&input).write_to_file(&path);
        assert!(result.is_err(), "accepted invalid input {input}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "previous valid output");
    }
}

#[test]
fn reproducible_manifest_and_node_scope() {
    let nodes = ["ECU"];
    let cfg = Config::builder()
        .dbc_name("fixture")
        .dbc_content(DBC)
        .selected_nodes(&nodes)
        .build();
    let a = cfg
        .generate_artifacts(DBC.as_bytes(), InputEncoding::Utf8)
        .unwrap();
    let b = cfg
        .generate_artifacts(DBC.as_bytes(), InputEncoding::Utf8)
        .unwrap();
    assert_eq!(a.code, b.code);
    assert_eq!(a.manifest, b.manifest);
    let manifest: serde_json::Value = serde_json::from_str(&a.manifest).unwrap();
    let messages = manifest["messages"].as_array().unwrap();
    assert!(messages.iter().any(|m| m["source_name"] == "RxOnly"));
    assert!(messages.iter().any(|m| {
        m["source_name"] == "Unrelated"
            && m["transmitters"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("ECU"))
    }));
    assert!(
        messages
            .iter()
            .any(|m| m["source_name"] == "DiagRequest" && m["type_name"] != "DiagRequest")
    );
    let dir = tempfile::tempdir().unwrap();
    a.write_to_directory(dir.path()).unwrap();
    assert!(
        cfg.cache_matches(dir.path(), DBC.as_bytes(), InputEncoding::Utf8)
            .unwrap()
    );
    fs::write(dir.path().join("messages.rs"), "tampered").unwrap();
    assert!(
        !cfg.cache_matches(dir.path(), DBC.as_bytes(), InputEncoding::Utf8)
            .unwrap()
    );
    a.write_to_directory(dir.path()).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("messages.rs")).unwrap(),
        a.code
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("manifest.json")).unwrap(),
        a.manifest
    );
    let receiver = ["Receiver"];
    let filtered = Config::builder()
        .dbc_name("fixture")
        .dbc_content(DBC)
        .selected_nodes(&receiver)
        .build()
        .generate_artifacts(DBC.as_bytes(), InputEncoding::Utf8)
        .unwrap();
    let filtered: serde_json::Value = serde_json::from_str(&filtered.manifest).unwrap();
    for m in filtered["messages"].as_array().unwrap() {
        assert!(m["source_name"] != "RxOnly" && m["source_name"] != "Unrelated");
        let original = messages
            .iter()
            .find(|v| v["source_name"] == m["source_name"])
            .unwrap();
        assert_eq!(original["type_name"], m["type_name"]);
    }
    let nodes = ["Missing"];
    assert!(
        Config::builder()
            .dbc_name("fixture")
            .dbc_content(DBC)
            .selected_nodes(&nodes)
            .build()
            .generate()
            .is_err()
    );
}

#[test]
fn encoding_is_explicit_and_strict() {
    assert_eq!(
        decode_input(b"\xef\xbb\xbfVERSION", InputEncoding::Utf8).unwrap(),
        "VERSION"
    );
    assert!(decode_input(b"\xff", InputEncoding::Utf8).is_err());
    assert_eq!(
        decode_input(b"\xe9", InputEncoding::Windows1252).unwrap(),
        "é"
    );
    let (korean, _, errors) = encoding_rs::EUC_KR.encode("한국어");
    assert!(!errors);
    assert_eq!(
        decode_input(&korean, InputEncoding::Cp949).unwrap(),
        "한국어"
    );
    assert!(decode_input(b"\x81", InputEncoding::Cp949).is_err());
    let dbc = "VERSION \"한글 시험\"\nNS_ :\nBS_:\nBU_: ECU\nBO_ 1 Korean: 1 ECU\n SG_ Value : 0|8@1+ (1,0) [0|255] \"온도\" ECU\nVAL_ 1 Value 0 \"꺼짐\" 1 \"켜짐\";\n";
    let (bytes, _, errors) = encoding_rs::EUC_KR.encode(dbc);
    assert!(!errors);
    let text = decode_input(&bytes, InputEncoding::Cp949).unwrap();
    let artifacts = config(&text)
        .generate_artifacts(&bytes, InputEncoding::Cp949)
        .unwrap();
    compile_and_run(&artifacts.code, "");
}

#[test]
fn publishing_rolls_back_existing_pair() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("messages.rs"), "previous").unwrap();
    fs::create_dir(dir.path().join("manifest.json")).unwrap();
    let artifacts = config(DBC)
        .generate_artifacts(DBC.as_bytes(), InputEncoding::Utf8)
        .unwrap();
    assert!(artifacts.write_to_directory(dir.path()).is_err());
    assert_eq!(
        fs::read_to_string(dir.path().join("messages.rs")).unwrap(),
        "previous"
    );
}
