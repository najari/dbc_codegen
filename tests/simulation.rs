use dbc_codegen::{Config, FeatureConfig, InputEncoding, RoundingPolicy, decode_input};
use std::{fs, path::PathBuf, process::Command};

const DBC: &str = include_str!("fixtures/simulation.dbc");
const EXTENDED: &str = include_str!("fixtures/extended_mux.dbc");

#[test]
fn extended_mux_runtime_and_no_std() {
    let code = config(EXTENDED).generate().unwrap();
    compile_and_run(
        &code,
        r#"
        let mut r = Ranged::try_from(&[3, 0xff, 0x55, 0xa5][..]).unwrap();
        assert!(r.a_is_active() && r.c_is_active() && !r.b_is_active());
        assert_eq!(r.a().unwrap(), 255); assert_eq!(r.c().unwrap(), 0x55);
        let old = *r.raw(); assert!(r.set_b(1).is_err()); assert!(r.b().is_err());
        assert!(r.set_b_raw_val(0).is_err()); assert_eq!(*r.raw(), old);
        assert_eq!(r.b_raw_val(), 255);
        assert_eq!(r.set_a_quantized(0).unwrap(), 0); assert_eq!(*r.raw(), [3,0,0x55,0xa5]);
        for key in [0,3,4,5] { r.select_switch(key).unwrap(); assert!(r.a_is_active()); }
        r.select_switch(6).unwrap(); assert!(!r.a_is_active() && r.c_is_active());
        r.select_switch(255).unwrap(); assert_eq!(r.switch(), -1); assert!(!r.a_is_active());
        let old = *r.raw(); assert!(r.select_switch(256).is_err()); assert_eq!(*r.raw(),old);
        r.select_switch(1).unwrap(); r.set_b(7).unwrap(); assert_eq!(r.b().unwrap(),7);
        assert_eq!(r.always(),0xa5);

        let mut n = Nested::try_from(&[1,0xff,0x44,0x55,0x66][..]).unwrap();
        assert!(n.alternate_is_active()); assert!(!n.child_is_active());
        let old = *n.raw(); assert!(n.select_child(0).is_err()); assert!(n.leaf1().is_err()); assert_eq!(*n.raw(),old);
        n.select_switch(2).unwrap(); n.select_child(0).unwrap();
        assert!(n.leaf0_is_active() && !n.leaf1_is_active()); assert_eq!(n.child().unwrap(),0);
        n.set_leaf0(0).unwrap(); assert_eq!(*n.raw(),[2,0xfe,0,0x55,0x66]);
        n.select_child(1).unwrap(); assert_eq!(n.child().unwrap(),-1);
        assert!(n.leaf1_is_active() && !n.leaf0_is_active()); n.set_leaf1(42).unwrap();
        assert_eq!(*n.raw(),[2,0xff,42,0x55,0x66]);
        n.select_switch(1).unwrap(); assert!(!n.leaf1_is_active()); assert_eq!(n.alternate().unwrap(),255);

        let mut d = Independent::try_from(&[0xfc,0xfd,11,22,0x5a][..]).unwrap();
        assert!(d.a0_is_active() && d.b1_is_active()); d.set_a0(0).unwrap();
        assert_eq!(*d.raw(),[0xfc,0xfd,0,22,0x5a]);
        d.select_sa(1).unwrap(); d.select_sb(2).unwrap();
        assert!(d.a1_is_active() && d.b2_is_active()); d.set_b2(0).unwrap();
        assert_eq!(*d.raw(),[0xfd,0xfe,0,0,0x5a]);

        let mut f = Floats::new().unwrap(); f.select_switch(1).unwrap();
        f.set_value(-1.5).unwrap(); assert_eq!(f.value().unwrap(),-1.5);
        assert_eq!(*f.raw(),[1,0,0,0xc0,0xbf,0]);
        let old = *f.raw(); assert!(f.set_value(f32::NAN).is_err()); assert_eq!(*f.raw(),old);
        f.select_switch(2).unwrap(); assert!(f.value().is_err());
        f.set_state(FloatsState::On).unwrap(); assert!(matches!(f.state().unwrap(),FloatsState::On));
        f.set_state_raw_val(255).unwrap(); assert!(matches!(f.state().unwrap(),FloatsState::Fault));

        let mut w = WideSwitch::new().unwrap(); w.select_switch(u64::MAX).unwrap();
        w.set_data(65).unwrap(); assert_eq!(w.data().unwrap(),65);
        assert_eq!(&w.raw()[..8], &[255;8]); w.select_switch(0).unwrap(); assert!(w.data().is_err());
    "#,
    );
}

#[test]
fn extended_mux_rejects_invalid_graphs_and_concurrent_overlap() {
    let cases = [
        (
            EXTENDED.replace("300 A Switch 0-0, 3-5", "300 A Missing 0-0, 3-5"),
            "missing signal",
        ),
        (
            EXTENDED.replace("300 A Switch 0-0, 3-5", "300 A Always 0-0, 3-5"),
            "not a selector",
        ),
        (
            EXTENDED.replace("300 A Switch 0-0, 3-5", "300 A Switch 5-3"),
            "reversed mux range",
        ),
        (
            EXTENDED.replace("302 A0 SA 0-0", "302 A0 SA 4-4"),
            "wire width",
        ),
        (
            EXTENDED.replace("301 Child Switch 2-2", "301 Child Child 0-0"),
            "self-referencing",
        ),
        (
            format!("{EXTENDED}SG_MUL_VAL_ 301 Switch Child 0-0;\n"),
            "cyclic mux",
        ),
        (
            format!("{EXTENDED}SG_MUL_VAL_ 302 A0 SB 0-0;\n"),
            "multiple parents",
        ),
        (
            EXTENDED.replace(" SG_ B1 m1 : 24|8", " SG_ B1 m1 : 16|8"),
            "overlap while active",
        ),
        (
            EXTENDED.replace("300 B Switch 1-1", "300 B Switch 3-3"),
            "overlap while active",
        ),
        (
            EXTENDED.replace("SG_MUL_VAL_ 302 A0 SA 0-0;", ""),
            "ambiguous multiplexor",
        ),
        (
            EXTENDED.replace("SG_ Switch M : 0|8@1- (1,0)", "SG_ Switch M : 0|8@1- (2,0)"),
            "unscaled integer",
        ),
        (
            format!("{EXTENDED}SG_MUL_VAL_ 999 A Switch 0-0;\n"),
            "missing message",
        ),
    ];
    for (input, expected) in cases {
        let error = config(&input).generate().unwrap_err();
        assert!(
            format!("{error:#}").contains(expected),
            "{expected}: {error:#}"
        );
    }
}

#[test]
fn extended_mux_manifest_naming_and_arbitrary() {
    let renamed = EXTENDED.replace(" SG_ Always : 24", " SG_ AIsActive : 24");
    let cfg = Config::builder()
        .dbc_name("extended")
        .dbc_content(&renamed)
        .impl_arbitrary(FeatureConfig::Always)
        .physical_f64(true)
        .build();
    let artifacts = cfg
        .generate_artifacts(renamed.as_bytes(), InputEncoding::Utf8)
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&artifacts.manifest).unwrap();
    let signals = manifest["messages"][0]["signals"].as_array().unwrap();
    assert_eq!(
        signals[1]["mux"]["parent"]["ranges"],
        serde_json::json!([{"min":0,"max":0},{"min":3,"max":5}])
    );
    assert_ne!(signals[4]["field_name"], "a_is_active");
    compile_and_run(
        &artifacts.code,
        r#"
        use arbitrary::{Arbitrary,Unstructured};
        let bytes = [0xa5;2048]; let mut u = Unstructured::new(&bytes);
        let r = Ranged::arbitrary(&mut u).unwrap();
        assert!(r.a_is_active() || r.b_is_active() || r.c_is_active());
        let n = Nested::arbitrary(&mut u).unwrap();
        assert!(n.alternate_is_active() || n.leaf0_is_active() || n.leaf1_is_active());
        let d = Independent::arbitrary(&mut u).unwrap();
        assert!(d.a0_is_active() || d.a1_is_active()); assert!(d.b1_is_active() || d.b2_is_active());
        let mut f = Floats::new().unwrap(); f.select_switch(1).unwrap();
        assert_eq!(f.set_value_quantized(1.5).unwrap(),1.5); assert_eq!(f.value().unwrap(),1.5);
    "#,
    );
    let dumped = EXTENDED.replace("Child m2M", "Child M");
    compile_and_run(
        &config(&dumped).generate().unwrap(),
        r#"
        let n = Nested::try_from(&[2,1,42,0,0][..]).unwrap();
        assert!(n.leaf1_is_active()); assert_eq!(n.leaf1().unwrap(),42);
    "#,
    );
    let unsigned_child = EXTENDED.replace("Child m2M : 8|1@1-", "Child m2M : 8|1@1+");
    compile_and_run(
        &config(&unsigned_child).generate().unwrap(),
        r#"
        let n = Nested::try_from(&[2,1,42,0,0][..]).unwrap();
        assert_eq!(n.child().unwrap(),1u8); assert_eq!(Nested::CHILD_MIN,0i128);
    "#,
    );
    let parent_rename = EXTENDED.replace("Switch", "raw");
    let nodes = ["RX"];
    let scoped = Config::builder()
        .dbc_name("renamed parent")
        .dbc_content(&parent_rename)
        .selected_nodes(&nodes)
        .build()
        .generate_artifacts(parent_rename.as_bytes(), InputEncoding::Utf8)
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&scoped.manifest).unwrap();
    let parent = manifest["messages"][0]["signals"][0]["field_name"]
        .as_str()
        .unwrap();
    assert_ne!(parent, "raw");
    assert_eq!(
        manifest["messages"][0]["signals"][1]["mux"]["parent"]["field_name"],
        parent
    );
    assert_eq!(manifest["messages"].as_array().unwrap().len(), 5);
    compile_and_run(
        &scoped.code,
        &format!(
            "let mut r = Ranged::new().unwrap(); r.select_{parent}(3).unwrap(); r.set_a(42).unwrap(); assert_eq!(r.a().unwrap(),42);"
        ),
    );
}

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
