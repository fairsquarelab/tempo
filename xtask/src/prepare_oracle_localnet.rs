//! Writes per-validator `oracle.toml` and `evm_oracle.hex` under a `generate-localnet` tree.
//!
//! EVM signing keys use the same test mnemonic as genesis account generation, at indices
//! `oracle_mnemonic_base + validator_index` (default base `40`, matching `tempo-e2e`).
//!
//! For the first three validators (sorted by listen port), HTTP sources are rotated so local traffic
//! hits different backends (see `OracleConfig` / `SourceKind` in `tempo-commonware-node`):
//!
//! 1. Frankfurter only — <https://www.frankfurter.dev>
//! 2. Pyth Hermes first, then Frankfurter for gaps.
//! 3. ExchangeRate-API via `open.er-api.com/v6` (no API key; `/latest/...` lives under `/v6`).
//!
//! Additional validators get the Frankfurter-only template.

use std::{
    fs,
    path::{Path, PathBuf},
};

use alloy::signers::local::MnemonicBuilder;
use clap::Parser;
use eyre::{Context as _, bail};

#[derive(Debug, Parser)]
pub(crate) struct PrepareOracleLocalnet {
    /// Output directory previously populated by `generate-localnet`.
    #[arg(long, value_name = "DIR")]
    localnet_dir: PathBuf,

    #[arg(
        short,
        long,
        default_value = "test test test test test test test test test test test junk"
    )]
    mnemonic: String,

    /// Mnemonic index of the first validator's oracle EVM key (`40` matches `ORACLE_EVM_MNEMONIC_BASE` in tempo-e2e).
    #[arg(long, default_value_t = 40)]
    oracle_mnemonic_base: u32,
}

impl PrepareOracleLocalnet {
    pub(crate) fn run(self) -> eyre::Result<()> {
        let dirs = list_validator_dirs(&self.localnet_dir)?;
        if dirs.is_empty() {
            bail!(
                "no validator subdirectories found under `{}` (expected `IP:PORT` folder names)",
                self.localnet_dir.display()
            );
        }

        // Hermes `price_feeds` IDs (FX spot). KRW/JPY match `base = USD` / `quote = KRW|JPY`.
        const PYTH_USD_KRW: &str = "e539120487c29b4defdf9a53d337316ea022a2688978a468f9efd847201be7e3";
        const PYTH_USD_JPY: &str = "ef2c98c804ba503c6a707e38be4dfbb16683775f195b091252bf24693042fd52";

        let oracle_toml_frankfurter = r#"poll_interval_ms = 500

[[sources]]
name = "frankfurter"
kind = "frankfurter"
base_url = "https://api.frankfurter.app"
timeout_ms = 15000
"#;

        let oracle_toml_pyth_then_frankfurter = format!(
            r#"poll_interval_ms = 500

[[sources]]
name = "pyth-hermes"
kind = "pyth_hermes"
base_url = "https://hermes.pyth.network"
timeout_ms = 15000
feed_ids = {{ 410 = "{pkrw}", 392 = "{pjpy}" }}

[[sources]]
name = "frankfurter"
kind = "frankfurter"
base_url = "https://api.frankfurter.app"
timeout_ms = 15000
"#,
            pkrw = PYTH_USD_KRW,
            pjpy = PYTH_USD_JPY,
        );

        let oracle_toml_open_er = r#"poll_interval_ms = 500

[[sources]]
name = "open-er-api"
kind = "exchangerate_api"
base_url = "https://open.er-api.com/v6"
timeout_ms = 15000
"#;

        for (i, dir) in dirs.iter().enumerate() {
            let path = self.localnet_dir.join(dir);
            let oracle_toml = match i {
                0 => oracle_toml_frankfurter,
                1 => oracle_toml_pyth_then_frankfurter.as_str(),
                2 => oracle_toml_open_er,
                _ => oracle_toml_frankfurter,
            };
            fs::write(path.join("oracle.toml"), oracle_toml)
                .wrap_err_with(|| format!("write oracle.toml in {}", path.display()))?;

            let sk = MnemonicBuilder::from_phrase_nth(
                &self.mnemonic,
                self.oracle_mnemonic_base + i as u32,
            )
            .credential()
            .to_bytes();
            let hex_key = format!("0x{}", const_hex::encode(sk));
            fs::write(path.join("evm_oracle.hex"), format!("{hex_key}\n"))
                .wrap_err_with(|| format!("write evm_oracle.hex in {}", path.display()))?;
        }

        eprintln!(
            "Prepared oracle.toml + evm_oracle.hex for {} validators under {}",
            dirs.len(),
            self.localnet_dir.display()
        );
        Ok(())
    }
}

/// Returns `generate-localnet` validator directory names sorted by consensus listen port (e.g. `127.0.0.1:8000`).
fn list_validator_dirs(localnet_dir: &Path) -> eyre::Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(localnet_dir).wrap_err("read localnet dir")? {
        let entry = entry.wrap_err("read localnet dir entry")?;
        let file_type = entry.file_type().wrap_err("file_type")?;
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // Match `IP:PORT` layout from generate-localnet.
        let Some((_, port_str)) = name.rsplit_once(':') else {
            continue;
        };
        let Ok(port) = port_str.parse::<u16>() else {
            continue;
        };
        entries.push((port, PathBuf::from(name)));
    }
    entries.sort_by_key(|(port, _)| *port);
    Ok(entries.into_iter().map(|(_, p)| p).collect())
}
