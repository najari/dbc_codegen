//! Generate and compile each local sample without changing the source files.
//! Run: cargo run --locked --example verify_samples -- dbc_samples artifacts/sample-report.json
use anyhow::{Context, Result, ensure};
use dbc_codegen::{Config, FeatureConfig, InputEncoding, decode_input};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = Path::new(args.get(1).map_or("dbc_samples", String::as_str));
    let report = Path::new(
        args.get(2)
            .map_or("artifacts/sample-report.json", String::as_str),
    );
    let mut samples: Vec<_> = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .map(walkdir::DirEntry::into_path)
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dbc")))
        .collect();
    samples.sort();
    ensure!(!samples.is_empty(), "no DBC samples found");
    let deps = std::env::current_exe()?
        .parent()
        .context("missing examples directory")?
        .parent()
        .context("missing debug directory")?
        .join("deps");
    let rlib = |name: &str| -> Result<PathBuf> {
        fs::read_dir(&deps)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                let f = p
                    .file_name()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_default();
                f.starts_with(&format!("lib{name}-")) && f.ends_with(".rlib")
            })
            .with_context(|| format!("missing {name} rlib"))
    };
    let bitvec = rlib("bitvec")?;
    let embedded_can = rlib("embedded_can")?;
    let temp = tempfile::tempdir()?;
    let mut entries = Vec::new();
    for (index, path) in samples.iter().enumerate() {
        let bytes = fs::read(path)?;
        let input_hash = hash(&bytes);
        // An explicit policy for the five known Windows-1252 corpus files.
        // All other files remain UTF-8 and decoding failures are reported.
        let encoding = if let Some(encoding) = args.get(3) {
            match encoding.as_str() {
                "utf8" => InputEncoding::Utf8,
                "windows1252" => InputEncoding::Windows1252,
                "cp949" => InputEncoding::Cp949,
                _ => anyhow::bail!("encoding must be utf8, windows1252 or cp949"),
            }
        } else if matches!(
            path.file_name().and_then(|s| s.to_str()),
            Some("abs.dbc" | "choices.dbc" | "cp1252.dbc" | "issue_63.dbc" | "vehicle.dbc")
        ) {
            InputEncoding::Windows1252
        } else {
            InputEncoding::Utf8
        };
        let result = (|| -> Result<serde_json::Value> {
            let decoded = decode_input(&bytes, encoding)?;
            let artifacts = Config::builder()
                .dbc_name(
                    path.file_name()
                        .context("missing file name")?
                        .to_str()
                        .context("non-Unicode path")?,
                )
                .dbc_content(&decoded)
                .impl_debug(FeatureConfig::Always)
                .allow_dead_code(true)
                .build()
                .generate_artifacts(&bytes, encoding)?;
            let source = temp.path().join(format!("sample_{index}.rs"));
            fs::write(&source, &artifacts.code)?;
            let output = Command::new("rustc")
                .args(["--edition=2024", "--crate-type=lib", "--emit=metadata"])
                .arg(&source)
                .arg("-L")
                .arg(format!("dependency={}", deps.display()))
                .arg("--extern")
                .arg(format!("bitvec={}", bitvec.display()))
                .arg("--extern")
                .arg(format!("embedded_can={}", embedded_can.display()))
                .arg("-o")
                .arg(temp.path().join("sample.rmeta"))
                .output()?;
            let manifest: serde_json::Value = serde_json::from_str(&artifacts.manifest)?;
            Ok(
                serde_json::json!({"generation": "passed", "compilation": if output.status.success() { "passed" } else { "failed" },
                "compiler_output": if output.status.success() { String::new() } else { String::from_utf8_lossy(&output.stderr).into_owned() },
                "manifest": manifest }),
            )
        })();
        let mut entry = match result {
            Ok(v) => v,
            Err(e) => serde_json::json!({"generation": "rejected", "error": format!("{e:#}")}),
        };
        let unchanged = hash(&fs::read(path)?) == input_hash;
        ensure!(unchanged, "source changed during check: {}", path.display());
        entry["path"] = serde_json::json!(path.to_string_lossy());
        entry["input_sha256"] = serde_json::json!(input_hash);
        entry["encoding"] = serde_json::json!(encoding);
        entry["source_unchanged"] = serde_json::json!(unchanged);
        println!(
            "{}/{} {} {}",
            index + 1,
            samples.len(),
            path.display(),
            if entry["generation"] == "rejected" {
                "rejected"
            } else if entry["compilation"] == "passed" {
                "compiled"
            } else {
                "COMPILE FAILED"
            }
        );
        entries.push(entry);
    }
    let compiled = entries
        .iter()
        .filter(|e| e["compilation"] == "passed")
        .count();
    let rejected = entries
        .iter()
        .filter(|e| e["generation"] == "rejected")
        .count();
    let failed = entries.len() - compiled - rejected;
    let rust = Command::new("rustc").arg("--version").output()?;
    let output = serde_json::json!({"rustc": String::from_utf8_lossy(&rust.stdout).trim(), "total": entries.len(), "compiled": compiled, "rejected": rejected, "compile_failed": failed, "samples": entries});
    if let Some(parent) = report.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(report, serde_json::to_string_pretty(&output)? + "\n")?;
    println!(
        "{} compiled; {} rejected; {} compile failures. Report: {}",
        compiled,
        rejected,
        failed,
        report.display()
    );
    ensure!(failed == 0, "accepted definitions failed compilation");
    Ok(())
}
