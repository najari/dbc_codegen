use std::{fs, process::Command};

#[test]
fn korean_paths_manifest_and_failed_generation_preserve_outputs() {
    let root = std::env::temp_dir().join(format!("dbc-codegen-한국어 시험-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("입력 파일.dbc");
    let output = root.join("출력 폴더");
    fs::create_dir_all(&output).unwrap();
    fs::write(
        &source,
        concat!(
            "\u{feff}",
            include_str!("../../tests/fixtures/simulation.dbc")
        ),
    )
    .unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_dbc-codegen"))
            .arg(&source)
            .arg(&output)
            .args([
                "--node",
                "ECU",
                "--physical-f64",
                "--rounding",
                "nearest-away",
            ])
            .output()
            .unwrap()
    };
    let result = run();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let before = fs::read(output.join("messages.rs")).unwrap();
    let manifest = fs::read(output.join("manifest.json")).unwrap();
    fs::write(&source, "invalid dbc").unwrap();
    assert!(!run().status.success());
    assert_eq!(fs::read(output.join("messages.rs")).unwrap(), before);
    assert_eq!(fs::read(output.join("manifest.json")).unwrap(), manifest);
    fs::remove_dir_all(root).unwrap();
}
