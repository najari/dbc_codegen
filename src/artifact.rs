use anyhow::{Context, Result, ensure};
use can_dbc::Dbc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{borrow::Cow, io::Write, path::Path};

use crate::{Config, preparation};

/// Explicit input character encoding; decoding never guesses or replaces errors.
#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputEncoding {
    /// UTF-8, optionally with a BOM.
    #[default]
    Utf8,
    /// Windows Western European encoding.
    Windows1252,
    /// Korean Windows encoding (WHATWG EUC-KR/CP949).
    Cp949,
}

/// Decode DBC bytes strictly using the chosen encoding.
pub fn decode_input(bytes: &[u8], encoding: InputEncoding) -> Result<Cow<'_, str>> {
    match encoding {
        InputEncoding::Utf8 => Ok(std::str::from_utf8(bytes)
            .context("input is not UTF-8; select an explicit legacy encoding")?
            .trim_start_matches('\u{feff}')
            .into()),
        InputEncoding::Windows1252 | InputEncoding::Cp949 => {
            let decoder = match encoding {
                InputEncoding::Windows1252 => encoding_rs::WINDOWS_1252,
                _ => encoding_rs::EUC_KR,
            };
            decoder
                .decode_without_bom_handling_and_without_replacement(bytes)
                .context("invalid bytes for selected encoding")
        }
    }
}

pub(crate) fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Validated Rust output and reproducibility manifest, prepared before publication.
pub struct GeneratedArtifacts {
    /// Generated Rust source.
    pub code: String,
    /// JSON manifest including original names, source/output hashes and options.
    pub manifest: String,
}

impl GeneratedArtifacts {
    /// Publish both files with rollback on an I/O error. The manifest's code hash
    /// also lets readers detect an interrupted two-file update.
    pub fn write_to_directory(&self, directory: impl AsRef<Path>) -> Result<()> {
        let directory = directory.as_ref();
        ensure!(directory.is_dir(), "output path must be a directory");
        let stage = tempfile::tempdir_in(directory)?;
        let files = [
            ("messages.rs", self.code.as_bytes()),
            ("manifest.json", self.manifest.as_bytes()),
        ];
        for (name, bytes) in files {
            let mut file = std::fs::File::create(stage.path().join(name))?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        let mut backups = Vec::new();
        let mut published = Vec::new();
        let result = (|| -> Result<()> {
            for (name, _) in files {
                let target = directory.join(name);
                if target.exists() {
                    ensure!(target.is_file(), "output target `{name}` is not a file");
                    std::fs::rename(&target, stage.path().join(format!("{name}.old")))?;
                    backups.push(name);
                }
            }
            for (name, _) in files {
                std::fs::rename(stage.path().join(name), directory.join(name))?;
                published.push(name);
            }
            Ok(())
        })();
        if let Err(error) = result {
            let mut recovery_errors = Vec::new();
            for name in published {
                if let Err(e) = std::fs::remove_file(directory.join(name)) {
                    recovery_errors.push(e.to_string());
                }
            }
            for name in backups {
                if let Err(e) = std::fs::rename(
                    stage.path().join(format!("{name}.old")),
                    directory.join(name),
                ) {
                    recovery_errors.push(e.to_string());
                }
            }
            if !recovery_errors.is_empty() {
                let recovery = stage.keep();
                return Err(error.context(format!(
                    "rollback incomplete ({recovery_errors:?}); saved output backups at {}",
                    recovery.display()
                )));
            }
            return Err(error);
        }
        Ok(())
    }
}

impl Config<'_> {
    /// Check an existing code/manifest pair against this complete configuration.
    /// This conservatively regenerates in memory; it never republishes files.
    pub fn cache_matches(
        &self,
        directory: impl AsRef<Path>,
        source_bytes: &[u8],
        encoding: InputEncoding,
    ) -> Result<bool> {
        let directory = directory.as_ref();
        let expected = self.generate_artifacts(source_bytes, encoding)?;
        let read = |path: &Path| -> Result<Option<String>> {
            match std::fs::read_to_string(path) {
                Ok(v) => Ok(Some(v)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            }
        };
        Ok(
            read(&directory.join("messages.rs"))?.as_deref() == Some(expected.code.as_str())
                && read(&directory.join("manifest.json"))?.as_deref()
                    == Some(expected.manifest.as_str()),
        )
    }

    /// Generate code and a manifest from the exact original input bytes.
    pub fn generate_artifacts(
        &self,
        source_bytes: &[u8],
        encoding: InputEncoding,
    ) -> Result<GeneratedArtifacts> {
        let decoded = decode_input(source_bytes, encoding)?;
        ensure!(
            decoded == self.dbc_content.trim_start_matches('\u{feff}'),
            "source bytes do not match Config::dbc_content"
        );
        let code = self.generate()?;
        let original = Dbc::try_from(decoded.as_ref()).map_err(|e| anyhow::anyhow!("{e}"))?;
        let (_, mappings) = preparation::prepare(&original, self)?;
        let feature = |f: &crate::FeatureConfig<'_>| match f {
            crate::FeatureConfig::Always => "always".to_owned(),
            crate::FeatureConfig::Never => "never".to_owned(),
            crate::FeatureConfig::Gated(g) => format!("feature:{g}"),
        };
        let manifest = serde_json::to_string_pretty(&serde_json::json!({
            "schema_version": 1, "dbc_name": self.dbc_name,
            "input_sha256": hash(source_bytes), "code_sha256": hash(code.as_bytes()),
            "generator_version": env!("CARGO_PKG_VERSION"),
            "generator_revision": env!("DBC_CODEGEN_REVISION"),
            "generator_source_sha256": hash(concat!(include_str!("lib.rs"), include_str!("utils.rs"), include_str!("signal_type.rs"), include_str!("preparation.rs"), include_str!("artifact.rs"), include_str!("numeric.rs"), include_str!("feature_config.rs"), include_str!("keywords.rs"), include_str!("pad.rs"), include_str!("../Cargo.lock")).as_bytes()),
            "options": {
                "encoding": encoding, "selected_nodes": self.selected_nodes,
                "physical_f64": self.physical_f64, "rounding": self.rounding,
                "check_ranges": feature(&self.check_ranges), "padding_bit_value": self.padding_bit_value,
                "allow_dead_code": self.allow_dead_code, "debug_prints": self.debug_prints,
                "impl_debug": feature(&self.impl_debug), "impl_defmt": feature(&self.impl_defmt),
                "impl_arbitrary": feature(&self.impl_arbitrary), "impl_serde": feature(&self.impl_serde),
                "impl_error": feature(&self.impl_error), "impl_embedded_can_frame": feature(&self.impl_embedded_can_frame),
                "attribute_structs": format!("{:?}", self.attribute_structs)
            },
            "policies": {
                "raw_setter": "wire-range-checked; bypasses physical min/max",
                "physical_special_values": "reject NaN and infinity; raw IEEE bits preserve all patterns",
                "zero_range": "[0|0] is enforced when check_ranges is enabled",
                "multiplexing": "single unsigned selector; extended/nested definitions rejected",
                "publication": "two-file rollback on I/O failure; verify code_sha256 before reuse"
            },
            "messages": mappings
        }))?;
        Ok(GeneratedArtifacts {
            code,
            manifest: manifest + "\n",
        })
    }
}
