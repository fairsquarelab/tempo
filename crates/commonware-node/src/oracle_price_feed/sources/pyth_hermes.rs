//! Pyth Network Hermes price source.
//!
//! Fetches prices from the Pyth Network Hermes REST API (free public endpoint).
//! <https://hermes.pyth.network>
//!
//! Each `Currency` is mapped to a Pyth feed ID internally. Default feed IDs are hardcoded
//! for known currencies; TOML `feed_ids` overrides can replace them.
//!
//! Price convention: the Pyth price has an `expo` (negative exponent).
//! We normalize to 6 decimal places via `FxPrice::from_pyth`.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;

use crate::oracle_price_feed::currency::Currency;
use crate::oracle_price_feed::fx_price::FxPrice;
use crate::oracle_price_feed::price_source::{PriceSource, PriceSourceError};

pub(crate) struct PythHermesSource {
    name: String,
    client: reqwest::Client,
    base_url: String,
    /// Currency → (Pyth feed ID, inverted).
    ///
    /// `inverted = true` means the Pyth feed reports the rate with that currency as the
    /// base (e.g. EUR/USD gives USD per EUR), so we take the reciprocal to convert to our
    /// convention of "units of that currency per 1 USD".
    feed_ids: HashMap<Currency, (String, bool)>,
}

/// Default Pyth Hermes feed IDs.
///
/// Each entry is `(feed_id_hex, inverted)`.
/// - `inverted = false`: feed already returns "units of currency per 1 USD" (our convention).
/// - `inverted = true`: feed returns "USD per unit of currency" — must be reciprocated.
///
/// EUR uses the Pyth EUR/USD feed which is quoted as EUR-base (1 EUR = X USD), so inverted.
fn default_feed_ids() -> HashMap<Currency, (String, bool)> {
    HashMap::from([
        (
            Currency::KRW,
            ("e539120487c29b4defdf9a53d337316ea022a2688978a468f9efd847201be7e3".to_string(), false),
        ),
        (
            Currency::JPY,
            ("ef2c98c804ba503c6a707e38be4dfbb16683775f195b091252bf24693042fd52".to_string(), false),
        ),
        (
            Currency::EUR,
            // EUR/USD: 1 EUR = X USD — reciprocate to get EUR per USD.
            ("a995d00bb36a63cef7fd2c287dc105fc8f3d93779f062f09551b0af3e81ec30b".to_string(), true),
        ),
        (
            Currency::GBP,
            // GBP/USD: 1 GBP = X USD — reciprocate to get GBP per USD.
            ("84c2dde9633d93d1bcad84e7dc41c9d56578b7ec52fabedc1f335d673df0a7c1".to_string(), true),
        ),
        (
            Currency::SGD,
            // USD/SGD: direct — already units of SGD per 1 USD.
            ("396a969a9c1480fa15ed50bc59149e2c0075a72fe8f458ed941ddec48bdb4918".to_string(), false),
        ),
        (
            Currency::CHF,
            // USD/CHF: direct — already units of CHF per 1 USD.
            ("0b1e3297e69f162877b577b0d6a47a0d63b2392bc8499e6540da4187a63e28f8".to_string(), false),
        ),
    ])
}

impl PythHermesSource {
    pub(crate) fn new(
        name: String,
        base_url: String,
        timeout_ms: u64,
        toml_overrides: HashMap<u32, String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .expect("failed to build reqwest client");

        // Start with defaults, apply TOML overrides (not inverted by default).
        let mut feed_ids = default_feed_ids();
        for (cid, feed_id) in toml_overrides {
            if let Ok(currency) = Currency::try_from(cid) {
                feed_ids.insert(currency, (normalize_pyth_id(&feed_id), false));
            }
        }

        Self {
            name,
            client,
            base_url,
            feed_ids,
        }
    }
}

#[derive(Deserialize)]
struct HermesResponse {
    parsed: Vec<HermesParsed>,
}

#[derive(Deserialize)]
struct HermesParsed {
    id: String,
    price: HermesPrice,
}

#[derive(Deserialize)]
struct HermesPrice {
    price: String,
    expo: i32,
}

#[async_trait]
impl PriceSource for PythHermesSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn fetch_prices(
        &self,
        currencies: &[Currency],
    ) -> Result<HashMap<Currency, FxPrice>, PriceSourceError> {
        // Only handle currencies that have a feed ID configured.
        let pyth_currencies: Vec<(Currency, &str, bool)> = currencies
            .iter()
            .filter_map(|c| {
                self.feed_ids
                    .get(c)
                    .map(|(id, inverted)| (*c, id.as_str(), *inverted))
            })
            .collect();

        if pyth_currencies.is_empty() {
            return Ok(HashMap::new());
        }

        // Build query: ids[]=0x...&ids[]=0x...
        let mut url = format!("{}/v2/updates/price/latest", self.base_url);
        let mut first = true;
        for (_, id, _) in &pyth_currencies {
            if first {
                url.push('?');
                first = false;
            } else {
                url.push('&');
            }
            url.push_str("ids[]=");
            url.push_str(id);
        }

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| PriceSourceError::Http(e.to_string()))?
            .json::<HermesResponse>()
            .await
            .map_err(|e| PriceSourceError::Parse(e.to_string()))?;

        // Index by normalized id (strip 0x prefix, lowercase).
        let by_id: HashMap<String, &HermesParsed> = resp
            .parsed
            .iter()
            .map(|p| (normalize_pyth_id(&p.id), p))
            .collect();

        let mut results = HashMap::new();
        for (currency, raw_id, inverted) in &pyth_currencies {
            let key = normalize_pyth_id(raw_id);
            if let Some(parsed) = by_id.get(&key) {
                if let Some(price) = FxPrice::from_pyth(&parsed.price.price, parsed.price.expo) {
                    let price = if *inverted { price.reciprocal() } else { Some(price) };
                    if let Some(price) = price {
                        results.insert(*currency, price);
                    }
                }
            }
        }

        Ok(results)
    }
}

fn normalize_pyth_id(id: &str) -> String {
    id.trim_start_matches("0x").to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_source(base_url: String) -> PythHermesSource {
        PythHermesSource::new("test".to_string(), base_url, 3000, HashMap::new())
    }

    #[test]
    fn from_pyth_expo_minus_5() {
        let s = FxPrice::from_pyth("145023000", -5).unwrap();
        assert_eq!(s.inner(), alloy_primitives::U256::from(1_450_230_000u128));
    }

    #[test]
    fn from_pyth_expo_minus_8() {
        let s = FxPrice::from_pyth("145023000", -8).unwrap();
        assert_eq!(s.inner(), alloy_primitives::U256::from(1_450_230u128));
    }

    #[test]
    fn from_pyth_negative_price() {
        assert!(FxPrice::from_pyth("-1", -6).is_none());
    }

    #[test]
    fn normalize_strips_0x_and_lowercases() {
        assert_eq!(normalize_pyth_id("0xABCDEF"), "abcdef");
        assert_eq!(normalize_pyth_id("abcdef"), "abcdef");
    }

    #[tokio::test]
    async fn fetch_prices_empty_currencies_returns_empty() {
        let source = make_source("http://localhost:9999".to_string());
        let result = source.fetch_prices(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn fetch_prices_parses_hermes_response() {
        let krw_feed = default_feed_ids()
            .get(&Currency::KRW)
            .cloned()
            .unwrap()
            .0;

        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/v2/updates/price/latest".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "parsed": [{
                        "id": &krw_feed,
                        "price": { "price": "145023000", "expo": -5 }
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let source = make_source(server.url());
        let result = source.fetch_prices(&[Currency::KRW]).await.unwrap();

        assert_eq!(
            result[&Currency::KRW].inner(),
            alloy_primitives::U256::from(1_450_230_000u128)
        );
    }

    #[tokio::test]
    async fn fetch_prices_handles_http_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(503)
            .create_async()
            .await;

        let source = make_source(server.url());
        let result = source.fetch_prices(&[Currency::KRW]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_prices_id_not_in_response_omits_currency() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"parsed":[]}"#)
            .create_async()
            .await;

        let source = make_source(server.url());
        let result = source.fetch_prices(&[Currency::KRW]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn fetch_prices_inverted_eur_feed_reciprocates() {
        // Simulate Pyth EUR/USD feed: 1 EUR = 1.15 USD → price=115000, expo=-5
        // Expected: reciprocal = 10^12 / 1_150_000 = 869_565 (0.869565 EUR/USD)
        let eur_feed = default_feed_ids()
            .get(&Currency::EUR)
            .cloned()
            .unwrap()
            .0;

        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/v2/updates/price/latest".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "parsed": [{
                        "id": &eur_feed,
                        "price": { "price": "115000", "expo": -5 }
                    }]
                })
                .to_string(),
            )
            .create_async()
            .await;

        let source = make_source(server.url());
        let result = source.fetch_prices(&[Currency::EUR]).await.unwrap();

        // 1.15 → FxPrice(1_150_000); reciprocal = 10^12 / 1_150_000 = 869_565
        let expected = alloy_primitives::U256::from(1_000_000_000_000u128 / 1_150_000u128);
        assert_eq!(result[&Currency::EUR].inner(), expected);
    }
}
