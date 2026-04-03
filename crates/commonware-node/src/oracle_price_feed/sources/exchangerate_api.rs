//! ExchangeRate-API price source.
//!
//! Fetches FX rates from ExchangeRate-API (free tier: 1,500 req/month).
//! <https://www.exchangerate-api.com>
//!
//! Without an API key: `GET {base_url}/latest/{base}` (e.g. open.er-api.com uses `{base_url}` = `https://open.er-api.com/v6`).
//! With an API key:    `GET {base_url}/{api_key}/latest/{base}`
//!
//! Price convention identical to Frankfurter:
//! `quote/base` — amount of `quote` per 1 `base`, 6 decimals.

use std::collections::HashMap;

use async_trait::async_trait;
use serde::Deserialize;

use crate::oracle_price_feed::currency::Currency;
use crate::oracle_price_feed::fx_price::FxPrice;
use crate::oracle_price_feed::price_source::{PriceSource, PriceSourceError};

pub(crate) struct ExchangeRateApiSource {
    name: String,
    client: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl ExchangeRateApiSource {
    pub(crate) fn new(
        name: String,
        base_url: String,
        timeout_ms: u64,
        api_key: Option<String>,
    ) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .build()
            .expect("failed to build reqwest client");
        Self {
            name,
            client,
            base_url,
            api_key,
        }
    }
}

#[derive(Deserialize)]
struct ExchangeRateResponse {
    /// Paid/v4 responses use `conversion_rates`; open.er-api.com v6 uses `rates`.
    #[serde(alias = "rates")]
    conversion_rates: HashMap<String, f64>,
}

#[async_trait]
impl PriceSource for ExchangeRateApiSource {
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
        let url = match &self.api_key {
            Some(key) => format!("{}/{key}/latest/USD", self.base_url),
            None => format!("{}/latest/USD", self.base_url),
        };

        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| PriceSourceError::Http(e.to_string()))?
            .json::<ExchangeRateResponse>()
            .await
            .map_err(|e| PriceSourceError::Parse(e.to_string()))?;

        let mut results = HashMap::new();
        for &currency in currencies {
            let symbol = currency.to_string();
            if let Some(&rate) = resp.conversion_rates.get(&symbol) {
                results.insert(currency, FxPrice::from_f64_rate(rate));
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_source(base_url: &str, api_key: Option<&str>) -> ExchangeRateApiSource {
        ExchangeRateApiSource::new(
            "test".to_string(),
            base_url.to_string(),
            3000,
            api_key.map(|s| s.to_string()),
        )
    }

    #[tokio::test]
    async fn fetch_prices_empty_currencies_returns_empty() {
        let source = make_source("http://localhost:9999", None);
        let result = source.fetch_prices(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn fetch_prices_without_api_key_uses_correct_url() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/latest/USD")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"conversion_rates":{"KRW":1450.23}}"#)
            .create_async()
            .await;

        let source = make_source(&server.url(), None);
        let result = source.fetch_prices(&[Currency::KRW]).await.unwrap();

        assert_eq!(
            result[&Currency::KRW].inner(),
            alloy_primitives::U256::from(1_450_230_000u128)
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_prices_with_api_key_uses_correct_url() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/mykey123/latest/USD")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"conversion_rates":{"KRW":1450.23}}"#)
            .create_async()
            .await;

        let source = make_source(&server.url(), Some("mykey123"));
        let result = source.fetch_prices(&[Currency::KRW]).await.unwrap();

        assert_eq!(
            result[&Currency::KRW].inner(),
            alloy_primitives::U256::from(1_450_230_000u128)
        );
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_prices_multiple_currencies() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/latest/USD")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"conversion_rates":{"KRW":1450.0,"JPY":155.5}}"#)
            .expect(1)
            .create_async()
            .await;

        let source = make_source(&server.url(), None);
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
            .with_status(429)
            .create_async()
            .await;

        let source = make_source(&server.url(), None);
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
            .with_body(r#"{"conversion_rates":{}}"#)
            .create_async()
            .await;

        let source = make_source(&server.url(), None);
        let result = source.fetch_prices(&[Currency::KRW]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn fetch_prices_accepts_rates_alias_like_open_er_api_v6() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/latest/USD")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"result":"success","rates":{"KRW":1500.0}}"#)
            .create_async()
            .await;

        let source = make_source(&server.url(), None);
        let result = source.fetch_prices(&[Currency::KRW]).await.unwrap();
        assert_eq!(
            result[&Currency::KRW].inner(),
            alloy_primitives::U256::from(1_500_000_000u128)
        );
    }
}
