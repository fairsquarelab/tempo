//! TOML configuration structures for the oracle price feed.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Top-level oracle configuration loaded from a TOML file.
#[derive(Debug, Clone, Deserialize)]
pub struct OracleConfig {
    /// Price sources in priority order (first = highest priority).
    #[serde(default)]
    pub sources: Vec<SourceConfig>,
    /// How often the oracle actor refreshes its price cache over HTTP (milliseconds).
    /// Prefer a value below block time so proposals often see a recent snapshot.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

impl OracleConfig {
    /// Load configuration from a TOML file.
    pub(crate) fn load(path: &Path) -> eyre::Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| eyre::eyre!("failed to read oracle config `{}`: {e}", path.display()))?;
        toml::from_str(&contents)
            .map_err(|e| eyre::eyre!("failed to parse oracle config `{}`: {e}", path.display()))
    }
}

/// Configuration for a single price source.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceConfig {
    /// Human-readable name used in logs.
    pub name: String,
    /// Which source implementation to use.
    pub kind: SourceKind,
    /// Base URL (without trailing slash).
    pub base_url: String,
    /// HTTP request timeout in milliseconds.
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    /// Optional API key (e.g. ExchangeRate-API free-tier key).
    pub api_key: Option<String>,
    /// Source-specific currency-to-feed mappings (e.g. Pyth feed IDs).
    /// Key: `currencyId` (ISO 4217 numeric as string), Value: source-specific identifier.
    /// TOML table keys are always strings, so we deserialize as `String` and convert to `u32`
    /// via [`SourceConfig::feed_ids_u32`].
    #[serde(default)]
    feed_ids_raw: HashMap<String, String>,
}

impl SourceConfig {
    /// Returns `feed_ids` with keys parsed from string to `u32`.
    pub fn feed_ids(&self) -> HashMap<u32, String> {
        self.feed_ids_raw
            .iter()
            .filter_map(|(k, v)| Some((k.parse::<u32>().ok()?, v.clone())))
            .collect()
    }
}

fn default_timeout_ms() -> u64 {
    3000
}

fn default_poll_interval_ms() -> u64 {
    500
}

/// The price source implementation to use.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Frankfurter (ECB-backed, free/unlimited). <https://www.frankfurter.dev>
    Frankfurter,
    /// Pyth Network Hermes REST API (blockchain-native, free public endpoint).
    PythHermes,
    /// ExchangeRate-API (free tier: 1,500 req/month). <https://www.exchangerate-api.com>
    ExchangerateApi,
    // TODO: Chainlink Data Streams
    // Requires API key and registered user access.
    // <https://docs.chain.link/data-streams>
    // Chainlink,
}
