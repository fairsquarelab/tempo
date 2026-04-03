//! Price source implementations.
//!
//! Each source fetches prices for a set of currency pairs and returns them as
//! on-chain `U256` values with 6 decimal places
//! (e.g. 1 USD = 1450.23 KRW → `U256(1_450_230_000)`).

// TODO: Chainlink Data Streams
// Requires API key and registered user access.
// <https://docs.chain.link/data-streams>
// pub mod chainlink;

pub(crate) mod exchangerate_api;
pub(crate) mod frankfurter;
pub(crate) mod pyth_hermes;
