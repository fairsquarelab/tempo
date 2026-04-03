//! Frankfurter price source.
//!
//! Fetches FX rates from the ECB-backed Frankfurter API (free, no API key required).
//! <https://www.frankfurter.dev>
//!
//! Price convention: `quote/base`, where the price is the amount of `quote` per 1 `base`.
//! Example: `KRW/USD` = "1 USD = 1450.23 KRW" → `FxPrice(1_450_230_000)` (6 decimals).

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;

use crate::oracle_price_feed::currency::Currency;
use crate::oracle_price_feed::fx_price::FxPrice;
use crate::oracle_price_feed::price_source::{PriceSource, PriceSourceError};

pub(crate) struct FrankfurterSource {
    name: String,
    client: reqwest::Client,
    base_url: String,
}

impl FrankfurterSource {
    pub(crate) fn new(name: String, base_url: String, timeout_ms: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .expect("failed to build reqwest client");
        Self {
            name,
            client,
            base_url,
        }
    }
}

#[derive(Deserialize)]
struct FrankfurterResponse {
    rates: HashMap<String, f64>,
}

#[async_trait]
impl PriceSource for FrankfurterSource {
    fn name(&self) -> &str {
        &self.name
    }

    async fn fetch_prices(
        &self,
        currencies: &[Currency],
    ) -> Result<HashMap<Currency, FxPrice>, PriceSourceError> {
        if currencies.is_empty() {
            return Ok(HashMap::new());
        }

        // All currencies are quoted against USD (the base currency).
        let symbols: Vec<String> = currencies.iter().map(|c| c.to_string()).collect();
        let symbols_param = symbols.join(",");

        let url = format!("{}/latest?base=USD&symbols={symbols_param}", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| PriceSourceError::Http(e.to_string()))?
            .json::<FrankfurterResponse>()
            .await
            .map_err(|e| PriceSourceError::Parse(e.to_string()))?;

        let mut results = HashMap::new();
        for &currency in currencies {
            let symbol = currency.to_string();
            if let Some(&rate) = resp.rates.get(&symbol) {
                results.insert(currency, FxPrice::from_f64_rate(rate));
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_f64_rate_typical() {
        let s = FxPrice::from_f64_rate(1450.23);
        assert_eq!(s.inner(), alloy_primitives::U256::from(1_450_230_000u128));
    }

    #[tokio::test]
    async fn fetch_prices_empty_currencies_returns_empty() {
        let source = FrankfurterSource::new(
            "test".to_string(),
            "http://localhost:9999".to_string(),
            1000,
        );
        let result = source.fetch_prices(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn fetch_prices_parses_response() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/latest?base=USD&symbols=KRW")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"rates":{"KRW":1450.23}}"#)
            .create_async()
            .await;

        let source = FrankfurterSource::new("test".to_string(), server.url(), 3000);
        let result = source.fetch_prices(&[Currency::KRW]).await.unwrap();

        assert_eq!(
            result[&Currency::KRW].inner(),
            alloy_primitives::U256::from(1_450_230_000u128)
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_prices_groups_multiple_symbols() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock(
                "GET",
                mockito::Matcher::Regex(r"^/latest\?base=USD&symbols=".to_string()),
            )
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"rates":{"KRW":1450.0,"JPY":155.5}}"#)
            .create_async()
            .await;

        let source = FrankfurterSource::new("test".to_string(), server.url(), 3000);
        let result = source
            .fetch_prices(&[Currency::KRW, Currency::JPY])
            .await
            .unwrap();

        assert_eq!(
            result[&Currency::KRW].inner(),
            alloy_primitives::U256::from(1_450_000_000u128)
        );
        assert_eq!(
            result[&Currency::JPY].inner(),
            alloy_primitives::U256::from(155_500_000u128)
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_prices_handles_http_error() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(500)
            .create_async()
            .await;

        let source = FrankfurterSource::new("test".to_string(), server.url(), 3000);
        let result = source.fetch_prices(&[Currency::KRW]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_prices_missing_quote_omits_currency() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", mockito::Matcher::Any)
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"rates":{}}"#)
            .create_async()
            .await;

        let source = FrankfurterSource::new("test".to_string(), server.url(), 3000);
        let result = source.fetch_prices(&[Currency::KRW]).await.unwrap();
        assert!(result.is_empty());
    }
}
