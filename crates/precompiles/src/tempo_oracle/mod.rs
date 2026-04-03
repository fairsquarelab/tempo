//! TempoOracle precompile — registered currency IDs and median updates from the block builder.
//!
//! Currency registry operations are controlled by `registry_admin`, set on the first successful
//! `registerCurrency`. The same admin may update `oracle_max_deviation_bps` and the feed quorum
//! (`setOracleMaxDeviationBps`, `setOracleFeedThreshold`). `setPrices` is restricted to the
//! current block beneficiary (coinbase).
//!
//! **`updatePriceFeed` / `setPriceFeed`**: calldata carries explicit `(currencyId, price)` tuples; the precompile
//! validates them and stores **one normalized pending row** (`Vec` of prices in registry order) under
//! `keccak256(abi.encode(uint256(blockNumber), address))` where `address` is the **effective oracle feed signer**
//! (VC2 `oracleFeedSigner`, or `validatorAddress` when that is zero) (see [`pending_feed_storage_key`]).
//! `setPriceFeed()` aggregates pending for the current block,
//! requires a quorum of active validators (threshold from storage, genesis-seeded default 2/3),
//! writes medians, then clears pending slots for active validators.
//!
//! Successful `setPriceFeed` emits one `tracing` **info** per registered currency (target `tempo_precompiles::tempo_oracle`):
//! block number, dense `currency_index`, `currency_id`, and median (`raw` + 6-decimal human form). `tempo.nu` default stdout
//! filter is `warn,tempo_precompiles::tempo_oracle=info` so these lines appear on the terminal without raising global
//! noise. Reth also writes `--log.file.directory` (e.g. `reth.log`).

pub mod dispatch;

use crate::error::{Result, TempoPrecompileError};
use crate::storage::{Handler, Mapping};
use crate::validator_config_v2::ValidatorConfigV2;
use alloy::primitives::{Address, B256, U256, keccak256};
use tempo_contracts::precompiles::{
    ITempoOracle, IValidatorConfigV2, TEMPO_ORACLE_ADDRESS, TempoOracleError,
};
use std::collections::HashMap;

use tempo_precompiles_macros::contract;
use tracing::info;

/// Fallback quorum when storage was never seeded (legacy genesis).
const DEFAULT_ORACLE_FEED_THRESHOLD_NUM: u64 = 2;
const DEFAULT_ORACLE_FEED_THRESHOLD_DEN: u64 = 3;

#[inline]
fn pending_feed_storage_key(block: u64, sender: Address) -> B256 {
    let mut enc = [0u8; 64];
    let bn = B256::from(U256::from(block));
    enc[0..32].copy_from_slice(bn.as_slice());
    enc[44..64].copy_from_slice(sender.as_slice());
    keccak256(enc).into()
}

#[inline]
fn effective_oracle_feed_signer(v: &IValidatorConfigV2::Validator) -> Address {
    if v.oracleFeedSigner.is_zero() {
        v.validatorAddress
    } else {
        v.oracleFeedSigner
    }
}

fn u256_to_u64(value: U256) -> Result<u64> {
    value
        .try_into()
        .map_err(|_| TempoOracleError::invalid_oracle_params().into())
}

fn median_u256(values: &mut Vec<U256>) -> Option<U256> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let n = values.len();
    Some(if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / U256::from(2u8)
    })
}

/// Human-readable oracle scalar: fixed-point with 6 decimals (fits `u128` for FX-style medians).
fn median_human_6dp(median: U256) -> String {
    let Ok(v) = u128::try_from(median) else {
        return median.to_string();
    };
    format!("{:.6}", v as f64 / 1_000_000.0)
}

/// On-chain oracle registry and median price storage.
#[contract(addr = TEMPO_ORACLE_ADDRESS)]
pub struct TempoOracle {
    /// Address allowed to add/remove currencies after initialization; set on first `registerCurrency`.
    registry_admin: Address,
    /// Number of registered currencies (also the next append index).
    currency_count: u64,
    /// Dense index → currency id (for `registeredCurrencyAt`).
    currency_ids: Mapping<u64, u32>,
    /// Currency id → index in `currency_ids` (only valid when registered).
    index_of: Mapping<u32, u64>,
    is_registered: Mapping<u32, bool>,
    /// Latest median price per currency (6-decimal FxPrice units; unset reads as 0).
    median_price: Mapping<u32, U256>,
    /// Pending price rows for `(block_number, sender)`; empty means no submission this block.
    pending_feeds: Mapping<B256, Vec<U256>>,
    /// Basis points band vs previous median in `setPriceFeed`; `0` disables filtering.
    oracle_max_deviation_bps: u32,
    /// Quorum numerator for `setPriceFeed` (≥ ceil(active × num / den) rows).
    oracle_feed_threshold_num: u64,
    /// Quorum denominator; must be non-zero after genesis `initialize_with_oracle_params`.
    oracle_feed_threshold_den: u64,
}

impl TempoOracle {
    /// Initializes bytecode marker and oracle params with defaults (bps `0`, threshold 2/3).
    pub fn initialize(&mut self) -> Result<()> {
        self.initialize_with_oracle_params(
            0,
            DEFAULT_ORACLE_FEED_THRESHOLD_NUM,
            DEFAULT_ORACLE_FEED_THRESHOLD_DEN,
        )
    }

    /// Seeds bytecode marker and oracle tuning parameters (typically from genesis `extra_fields`).
    pub fn initialize_with_oracle_params(
        &mut self,
        oracle_max_deviation_bps: u32,
        oracle_feed_threshold_num: u64,
        oracle_feed_threshold_den: u64,
    ) -> Result<()> {
        if oracle_feed_threshold_den == 0 {
            return Err(TempoPrecompileError::Fatal(
                "TempoOracle: oracle_feed_threshold_den must be non-zero".into(),
            ));
        }
        self.__initialize()?;
        self.oracle_max_deviation_bps
            .write(oracle_max_deviation_bps)?;
        self.oracle_feed_threshold_num
            .write(oracle_feed_threshold_num)?;
        self.oracle_feed_threshold_den
            .write(oracle_feed_threshold_den)?;
        Ok(())
    }

    fn min_feeds_required(&self, active: u64) -> Result<u64> {
        if active == 0 {
            return Ok(0);
        }
        let mut num = self.oracle_feed_threshold_num.read()?;
        let mut den = self.oracle_feed_threshold_den.read()?;
        if den == 0 {
            num = DEFAULT_ORACLE_FEED_THRESHOLD_NUM;
            den = DEFAULT_ORACLE_FEED_THRESHOLD_DEN;
        }
        Ok((active * num + den - 1) / den)
    }

    /// Returns the number of registered currencies.
    pub fn currency_count(&self) -> Result<U256> {
        Ok(U256::from(self.currency_count.read()?))
    }

    /// Returns the currency id at `index`, or panics out-of-bounds (reverts with panic ABI).
    pub fn registered_currency_at(&self, index: U256) -> Result<u32> {
        let idx: u64 = index
            .try_into()
            .map_err(|_| TempoPrecompileError::array_oob())?;
        let count = self.currency_count.read()?;
        if idx >= count {
            return Err(TempoPrecompileError::array_oob().into());
        }
        self.currency_ids[idx].read()
    }

    /// Whether `currency_id` is registered.
    pub fn is_currency_registered(&self, currency_id: u32) -> Result<bool> {
        self.is_registered[currency_id].read()
    }

    /// Registers `currency_id`. First successful call assigns `registry_admin` to `sender`.
    pub fn register_currency(&mut self, sender: Address, currency_id: u32) -> Result<()> {
        if self.is_registered[currency_id].read()? {
            return Ok(());
        }

        let admin = self.registry_admin.read()?;
        if admin.is_zero() {
            self.registry_admin.write(sender)?;
        } else if admin != sender {
            return Err(TempoOracleError::unauthorized().into());
        }

        let idx = self.currency_count.read()?;
        let new_count = idx
            .checked_add(1)
            .ok_or_else(TempoPrecompileError::under_overflow)?;

        self.currency_ids[idx].write(currency_id)?;
        self.index_of[currency_id].write(idx)?;
        self.is_registered[currency_id].write(true)?;
        self.currency_count.write(new_count)?;

        Ok(())
    }

    /// Removes `currency_id` from the registry (swap-remove). Admin-only.
    pub fn unregister_currency(&mut self, sender: Address, currency_id: u32) -> Result<()> {
        self.ensure_registry_admin(sender)?;

        if !self.is_registered[currency_id].read()? {
            return Err(TempoOracleError::currency_not_registered().into());
        }

        let idx = self.index_of[currency_id].read()?;
        let count = self.currency_count.read()?;
        let last = count
            .checked_sub(1)
            .ok_or_else(TempoPrecompileError::under_overflow)?;

        if idx != last {
            let moved = self.currency_ids[last].read()?;
            self.currency_ids[idx].write(moved)?;
            self.index_of[moved].write(idx)?;
        }

        self.currency_ids[last].delete()?;
        self.index_of[currency_id].delete()?;
        self.is_registered[currency_id].write(false)?;
        self.median_price[currency_id].delete()?;
        self.currency_count.write(last)?;

        Ok(())
    }

    /// Updates medians and the feeds root. Only the block beneficiary may call.
    pub fn set_prices(
        &mut self,
        sender: Address,
        call: ITempoOracle::setPricesCall,
    ) -> Result<()> {
        let beneficiary = self.storage.beneficiary();
        if sender != beneficiary {
            return Err(TempoOracleError::unauthorized().into());
        }

        if call.currencyIds.len() != call.medians.len() {
            return Err(TempoOracleError::arrays_length_mismatch().into());
        }

        for (currency_id, price) in call.currencyIds.iter().zip(call.medians.iter()) {
            let cid: u32 = (*currency_id)
                .try_into()
                .map_err(|_| TempoOracleError::invalid_oracle_params())?;
            if !self.is_registered[cid].read()? {
                return Err(TempoOracleError::currency_not_registered().into());
            }
            self.median_price[cid].write(*price)?;
        }

        Ok(())
    }

    /// One pending price row per registered currency index; active validator only; duplicate per block reverts.
    pub fn update_price_feed(
        &mut self,
        sender: Address,
        call: ITempoOracle::updatePriceFeedCall,
    ) -> Result<()> {
        let vc = ValidatorConfigV2::new();
        vc.validator_by_oracle_feed_signer(sender)?;

        let count = self.currency_count.read()?;
        let expect = count as usize;
        if call.updates.len() != expect {
            return Err(TempoOracleError::arrays_length_mismatch().into());
        }

        let mut by_id: HashMap<u32, U256> = HashMap::with_capacity(expect);
        for u in call.updates.iter() {
            let currency_id = u.currencyId;
            if !self.is_registered[currency_id].read()? {
                return Err(TempoOracleError::currency_not_registered().into());
            }
            if by_id.insert(currency_id, u.price).is_some() {
                return Err(TempoOracleError::invalid_oracle_params().into());
            }
        }

        let mut row = Vec::with_capacity(expect);
        for idx in 0..count {
            let currency_id = self.currency_ids[idx].read()?;
            let price = by_id
                .get(&currency_id)
                .copied()
                .ok_or_else(|| TempoOracleError::invalid_oracle_params())?;
            row.push(price);
        }

        let bn = self.storage.block_number();
        let key = pending_feed_storage_key(bn, sender);
        let existing = self.pending_feeds[key].read()?;
        if !existing.is_empty() {
            return Err(TempoOracleError::duplicate_oracle_feed_update().into());
        }

        self.pending_feeds[key].write(row)?;
        Ok(())
    }

    /// Aggregate pending for this block (quorum), write medians, clear pending for active validators.
    pub fn set_price_feed(
        &mut self,
        sender: Address,
        call: ITempoOracle::setPriceFeedCall,
    ) -> Result<()> {
        let _ = call;
        let beneficiary = self.storage.beneficiary();
        if sender != beneficiary {
            return Err(TempoOracleError::unauthorized().into());
        }

        let vc = ValidatorConfigV2::new();
        let active = vc.get_active_validators()?;
        let required = self.min_feeds_required(active.len() as u64)?;
        let bn = self.storage.block_number();

        let currency_count = self.currency_count.read()? as usize;
        if currency_count == 0 {
            for v in &active {
                let signer = effective_oracle_feed_signer(v);
                let key = pending_feed_storage_key(bn, signer);
                self.pending_feeds[key].delete()?;
            }
            return Ok(());
        }

        let mut rows: Vec<Vec<U256>> = Vec::new();
        for v in &active {
            let signer = effective_oracle_feed_signer(v);
            let key = pending_feed_storage_key(bn, signer);
            let row = self.pending_feeds[key].read()?;
            if !row.is_empty() {
                rows.push(row);
            }
        }

        if (rows.len() as u64) < required {
            return Err(TempoOracleError::insufficient_feeds().into());
        }

        for currency_idx in 0..currency_count {
            let currency_id = self.currency_ids[currency_idx as u64].read()?;
            let mut col: Vec<U256> = rows.iter().map(|r| r[currency_idx]).collect();

            let max_bps = self.oracle_max_deviation_bps.read()?;
            if max_bps > 0 {
                let prev = self.median_price[currency_id].read()?;
                if !prev.is_zero() {
                    let bps = U256::from(max_bps);
                    let ten_k = U256::from(10_000u32);
                    let lower = prev * (ten_k.saturating_sub(bps)) / ten_k;
                    let upper = prev * (ten_k + bps) / ten_k;
                    col.retain(|p| *p >= lower && *p <= upper);
                }
            }

            let median =
                median_u256(&mut col).ok_or_else(|| TempoOracleError::insufficient_feeds())?;
            self.median_price[currency_id].write(median)?;

            let human = median_human_6dp(median);
            info!(
                target: "tempo_precompiles::tempo_oracle",
                block = bn,
                currency_index = currency_idx,
                currency_id = currency_id,
                raw = %median,
                human_6dp = %human,
                "setPriceFeed committed median"
            );
        }

        for v in &active {
            let signer = effective_oracle_feed_signer(v);
            let key = pending_feed_storage_key(bn, signer);
            self.pending_feeds[key].delete()?;
        }

        Ok(())
    }

    /// Returns the latest median for `currency_id` (0 if never set).
    pub fn get_oracle_price(&self, currency_id: u32) -> Result<U256> {
        self.median_price[currency_id].read()
    }

    /// Returns all registered currency IDs as a Vec.
    pub fn get_currencies(&self) -> Result<Vec<u32>> {
        let count = self.currency_count.read()?;
        let mut ids = Vec::with_capacity(count as usize);
        for idx in 0..count {
            ids.push(self.currency_ids[idx].read()?);
        }
        Ok(ids)
    }

    /// Cross rate: base/quote = price[base] * SCALE / price[quote].
    /// Reverts CurrencyNotRegistered if either is not registered.
    /// Returns 0 if either price is not set.
    pub fn get_pair_price(&self, base: u32, quote: u32) -> Result<U256> {
        if !self.is_registered[base].read()? {
            return Err(TempoOracleError::currency_not_registered().into());
        }
        if !self.is_registered[quote].read()? {
            return Err(TempoOracleError::currency_not_registered().into());
        }
        let base_price = self.median_price[base].read()?;
        let quote_price = self.median_price[quote].read()?;
        if base_price.is_zero() || quote_price.is_zero() {
            return Ok(U256::ZERO);
        }
        const SCALE: u128 = 1_000_000;
        Ok((base_price * U256::from(SCALE)) / quote_price)
    }

    /// View: current `oracle_max_deviation_bps`.
    pub fn read_oracle_max_deviation_bps(&self) -> Result<U256> {
        Ok(U256::from(self.oracle_max_deviation_bps.read()?))
    }

    /// View: quorum `(num, den)` used by `setPriceFeed`.
    pub fn read_oracle_feed_threshold(&self) -> Result<ITempoOracle::oracleFeedThresholdReturn> {
        let num = self.oracle_feed_threshold_num.read()?;
        let den = self.oracle_feed_threshold_den.read()?;
        Ok(ITempoOracle::oracleFeedThresholdReturn {
            num: U256::from(num),
            den: U256::from(den),
        })
    }

    /// Registry admin: update median band bps (`0` disables banding).
    pub fn set_oracle_max_deviation_bps(
        &mut self,
        sender: Address,
        call: ITempoOracle::setOracleMaxDeviationBpsCall,
    ) -> Result<()> {
        self.ensure_registry_admin(sender)?;
        self.oracle_max_deviation_bps.write(call.newBps)?;
        Ok(())
    }

    /// Registry admin: update feed quorum fraction.
    pub fn set_oracle_feed_threshold(
        &mut self,
        sender: Address,
        call: ITempoOracle::setOracleFeedThresholdCall,
    ) -> Result<()> {
        self.ensure_registry_admin(sender)?;
        let num = u256_to_u64(call.num)?;
        let den = u256_to_u64(call.den)?;
        if den == 0 || num == 0 || num > den {
            return Err(TempoOracleError::invalid_oracle_params().into());
        }
        self.oracle_feed_threshold_num.write(num)?;
        self.oracle_feed_threshold_den.write(den)?;
        Ok(())
    }

    fn ensure_registry_admin(&self, sender: Address) -> Result<()> {
        let admin = self.registry_admin.read()?;
        if admin.is_zero() || admin != sender {
            return Err(TempoOracleError::unauthorized().into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod update_price_feed_tests {
    use super::*;
    use alloy::sol_types::SolCall;

    #[test]
    fn update_price_feed_abi_roundtrip() {
        let updates = vec![
            ITempoOracle::OracleCurrencyPrice {
                currencyId: 410,
                price: U256::from(100u64),
            },
            ITempoOracle::OracleCurrencyPrice {
                currencyId: 392,
                price: U256::from(200u64),
            },
        ];
        let call = ITempoOracle::updatePriceFeedCall { updates };
        let encoded = call.abi_encode();
        let decoded = ITempoOracle::updatePriceFeedCall::abi_decode(&encoded).expect("decode");
        assert_eq!(decoded.updates.len(), 2);
        assert_eq!(decoded.updates[0].currencyId, 410);
        assert_eq!(decoded.updates[0].price, U256::from(100u64));
        assert_eq!(decoded.updates[1].currencyId, 392);
        assert_eq!(decoded.updates[1].price, U256::from(200u64));
    }

    #[test]
    fn update_price_feed_reorders_to_registry_row() {
        // Calldata order reversed vs registry order (410 at index 0, 392 at index 1).
        let updates = vec![
            ITempoOracle::OracleCurrencyPrice {
                currencyId: 392,
                price: U256::from(999u64),
            },
            ITempoOracle::OracleCurrencyPrice {
                currencyId: 410,
                price: U256::from(111u64),
            },
        ];

        let mut by_id: HashMap<u32, U256> = HashMap::new();
        for u in &updates {
            assert!(by_id.insert(u.currencyId, u.price).is_none());
        }
        let mut row = Vec::new();
        for id in [410u32, 392] {
            row.push(*by_id.get(&id).expect("missing currency"));
        }
        assert_eq!(row, vec![U256::from(111u64), U256::from(999u64)]);
    }
}
