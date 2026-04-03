//! `PriceSource` trait and `FalloverFetcher` (sequential source priority).

use std::collections::HashMap;

use async_trait::async_trait;
use tracing::{debug, warn};

use crate::oracle_price_feed::config::{OracleConfig, SourceKind};
use crate::oracle_price_feed::currency::Currency;
use crate::oracle_price_feed::fx_price::FxPrice;
use crate::oracle_price_feed::sources::{
    exchangerate_api::ExchangeRateApiSource, frankfurter::FrankfurterSource,
    pyth_hermes::PythHermesSource,
};

/// Error variants returned by price source implementations.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PriceSourceError {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("parse error: {0}")]
    Parse(String),
}

/// Trait implemented by each price source backend.
#[async_trait]
pub(crate) trait PriceSource: Send + Sync {
    /// Human-readable name used in log messages.
    fn name(&self) -> &str;

    /// Fetch prices for the given currencies.
    ///
    /// Returns a map from `Currency` to `FxPrice` (6-decimal fixed-point).
    /// May return a partial map if only some currencies are supported or available.
    async fn fetch_prices(
        &self,
        currencies: &[Currency],
    ) -> Result<HashMap<Currency, FxPrice>, PriceSourceError>;
}

/// Sequential fallover fetcher.
///
/// Tries each source in priority order.  For every currency it collects results
/// from the first source that returns a value for that currency; remaining currencies
/// are forwarded to the next source.
///
/// If no source can supply a price for a given currency the currency is omitted from
/// the final result (the caller should skip those currencies in the update calldata).
pub(crate) struct FalloverFetcher {
    sources: Vec<Box<dyn PriceSource>>,
}

impl FalloverFetcher {
    /// Build a `FalloverFetcher` from an [`OracleConfig`].
    pub(crate) fn from_config(config: &OracleConfig) -> Self {
        let sources: Vec<Box<dyn PriceSource>> = config
            .sources
            .iter()
            .map(|s| -> Box<dyn PriceSource> {
                match s.kind {
                    SourceKind::Frankfurter => Box::new(FrankfurterSource::new(
                        s.name.clone(),
                        s.base_url.clone(),
                        s.timeout_ms,
                    )),
                    SourceKind::PythHermes => Box::new(PythHermesSource::new(
                        s.name.clone(),
                        s.base_url.clone(),
                        s.timeout_ms,
                        s.feed_ids(),
                    )),
                    SourceKind::ExchangerateApi => Box::new(ExchangeRateApiSource::new(
                        s.name.clone(),
                        s.base_url.clone(),
                        s.timeout_ms,
                        s.api_key.clone(),
                    )),
                }
            })
            .collect();

        Self { sources }
    }

    /// Fetch prices for the given currencies, returning a partial map.
    /// Missing currencies are absent (the caller must not submit zero for them).
    pub(crate) async fn fetch_prices(
        &self,
        currencies: &[Currency],
    ) -> HashMap<Currency, FxPrice> {
        if self.sources.is_empty() || currencies.is_empty() {
            return HashMap::new();
        }

        let mut remaining: Vec<Currency> = currencies.to_vec();
        let mut collected: HashMap<Currency, FxPrice> = HashMap::new();

        for source in &self.sources {
            if remaining.is_empty() {
                break;
            }

            match source.fetch_prices(&remaining).await {
                Ok(prices) => {
                    let before = remaining.len();
                    remaining.retain(|c| {
                        if let Some(scalar) = prices.get(c) {
                            collected.insert(*c, *scalar);
                            false
                        } else {
                            true
                        }
                    });
                    let fetched = before - remaining.len();
                    debug!(
                        source = source.name(),
                        fetched,
                        still_missing = remaining.len(),
                        "oracle price fetch"
                    );
                }
                Err(e) => {
                    warn!(source = source.name(), error = %e, "oracle price source failed");
                }
            }
        }

        if !remaining.is_empty() {
            warn!(
                missing = remaining.len(),
                currencies = ?remaining,
                "oracle prices unavailable for some currencies"
            );
        }

        collected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::U256;

    struct MockSource {
        name: String,
        prices: HashMap<Currency, FxPrice>,
        fail: bool,
    }

    impl MockSource {
        fn ok(name: &str, prices: HashMap<Currency, FxPrice>) -> Box<Self> {
            Box::new(Self {
                name: name.to_string(),
                prices,
                fail: false,
            })
        }

        fn failing(name: &str) -> Box<Self> {
            Box::new(Self {
                name: name.to_string(),
                prices: HashMap::new(),
                fail: true,
            })
        }
    }

    #[async_trait::async_trait]
    impl PriceSource for MockSource {
        fn name(&self) -> &str {
            &self.name
        }

        async fn fetch_prices(
            &self,
            _currencies: &[Currency],
        ) -> Result<HashMap<Currency, FxPrice>, PriceSourceError> {
            if self.fail {
                return Err(PriceSourceError::Http("mock failure".into()));
            }
            Ok(self.prices.clone())
        }
    }

    fn fetcher(sources: Vec<Box<dyn PriceSource>>) -> FalloverFetcher {
        FalloverFetcher { sources }
    }

    fn scalar(v: u128) -> FxPrice {
        FxPrice::from_raw(U256::from(v))
    }

    #[tokio::test]
    async fn no_sources_returns_empty() {
        let f = fetcher(vec![]);
        let result = f.fetch_prices(&[Currency::KRW]).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn empty_currencies_returns_empty() {
        let f = fetcher(vec![]);
        let result = f.fetch_prices(&[]).await;
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn single_source_returns_price() {
        let prices = HashMap::from([(Currency::KRW, scalar(1_450_000_000))]);
        let f = fetcher(vec![MockSource::ok("s1", prices)]);
        let result = f.fetch_prices(&[Currency::KRW]).await;
        assert_eq!(result[&Currency::KRW], scalar(1_450_000_000));
    }

    #[tokio::test]
    async fn first_source_failure_falls_over_to_second() {
        let prices = HashMap::from([(Currency::KRW, scalar(1_450_000_000))]);
        let f = fetcher(vec![
            MockSource::failing("bad"),
            MockSource::ok("good", prices),
        ]);
        let result = f.fetch_prices(&[Currency::KRW]).await;
        assert_eq!(result[&Currency::KRW], scalar(1_450_000_000));
    }

    #[tokio::test]
    async fn first_source_fills_partial_second_fills_rest() {
        let prices1 = HashMap::from([(Currency::KRW, scalar(1_000_000))]);
        let prices2 = HashMap::from([(Currency::JPY, scalar(2_000_000))]);
        let f = fetcher(vec![
            MockSource::ok("s1", prices1),
            MockSource::ok("s2", prices2),
        ]);
        let result = f.fetch_prices(&[Currency::KRW, Currency::JPY]).await;
        assert_eq!(result[&Currency::KRW], scalar(1_000_000));
        assert_eq!(result[&Currency::JPY], scalar(2_000_000));
    }

    #[tokio::test]
    async fn first_source_wins_second_not_called_for_resolved() {
        let prices1 = HashMap::from([(Currency::KRW, scalar(1_000_000))]);
        let prices2 = HashMap::from([(Currency::KRW, scalar(9_999_999))]);
        let f = fetcher(vec![
            MockSource::ok("s1", prices1),
            MockSource::ok("s2", prices2),
        ]);
        let result = f.fetch_prices(&[Currency::KRW]).await;
        assert_eq!(result[&Currency::KRW], scalar(1_000_000));
    }

    #[tokio::test]
    async fn all_sources_fail_returns_empty() {
        let f = fetcher(vec![
            MockSource::failing("s1"),
            MockSource::failing("s2"),
        ]);
        let result = f.fetch_prices(&[Currency::KRW]).await;
        assert!(result.is_empty());
    }
}
