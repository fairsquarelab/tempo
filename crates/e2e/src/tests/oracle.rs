//! Live oracle E2E: three validators, three different HTTP price sources.
//!
//! Run manually (requires outbound HTTPS):
//! `cargo test -p tempo-e2e oracle_price_feed_pipeline_live_apis -- --ignored --nocapture`

use alloy_primitives::Address;
use commonware_macros::test_traced;
use std::collections::HashMap;
use tempo_commonware_node::oracle_price_feed::config::{OracleConfig, SourceConfig, SourceKind};

use crate::{Setup, run};

/// Pyth Hermes `FX.USD/KRW` feed (USDKRW spot offshore). See Hermes `price_feeds` API.
const PYTH_USD_KRW_FEED_ID: &str =
    "e539120487c29b4defdf9a53d337316ea022a2688978a468f9efd847201be7e3";

/// ISO 4217: KRW = 410
const KRW: u32 = 410;

#[test_traced]
#[ignore = "live HTTPS (Frankfurter, Hermes, open.er-api.com); cargo test -p tempo-e2e oracle_price_feed_pipeline_live_apis -- --ignored"]
fn oracle_price_feed_pipeline_live_apis() {
    let _ = tempo_eyre::install();

    let pyth_id = std::env::var("TEMPO_E2E_PYTH_PRICE_ID_KRW_USD")
        .unwrap_or_else(|_| PYTH_USD_KRW_FEED_ID.to_string());

    let configs = vec![
        OracleConfig {
            sources: vec![SourceConfig {
                name: "frankfurter".into(),
                kind: SourceKind::Frankfurter,
                base_url: "https://api.frankfurter.app".into(),
                timeout_ms: 15_000,
                api_key: None,
                feed_ids: HashMap::new(),
            }],
            poll_interval_ms: 500,
        },
        OracleConfig {
            sources: vec![SourceConfig {
                name: "pyth-hermes".into(),
                kind: SourceKind::PythHermes,
                base_url: "https://hermes.pyth.network".into(),
                timeout_ms: 15_000,
                api_key: None,
                feed_ids: HashMap::from([(KRW, pyth_id)]),
            }],
            poll_interval_ms: 500,
        },
        OracleConfig {
            sources: vec![SourceConfig {
                name: "open-er-api".into(),
                kind: SourceKind::ExchangerateApi,
                base_url: "https://open.er-api.com/v6".into(),
                timeout_ms: 15_000,
                api_key: None,
                feed_ids: HashMap::new(),
            }],
            poll_interval_ms: 500,
        },
    ];

    let admin = Address::random();
    let setup = Setup::new()
        .how_many_signers(3)
        .oracle_currencies(vec![KRW])
        .oracle_registry_admin(admin)
        .oracle_configs_per_validator(configs);

    run(setup, |metric, value| {
        metric.ends_with("_marshal_processed_height")
            && value.parse::<u64>().unwrap_or(0) >= 8
    });
}
