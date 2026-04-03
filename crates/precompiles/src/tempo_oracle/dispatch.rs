//! ABI dispatch for the [`TempoOracle`] precompile.

use crate::{
    Precompile, dispatch_call, input_cost, metadata, mutate_void, tempo_oracle::TempoOracle, view,
};
use alloy::{primitives::Address, sol_types::SolInterface};
use revm::precompile::{PrecompileError, PrecompileResult};
use tempo_contracts::precompiles::{ITempoOracle, ITempoOracle::ITempoOracleCalls};

impl Precompile for TempoOracle {
    fn call(&mut self, calldata: &[u8], msg_sender: Address) -> PrecompileResult {
        self.storage
            .deduct_gas(input_cost(calldata.len()))
            .map_err(|_| PrecompileError::OutOfGas)?;

        dispatch_call(calldata, ITempoOracleCalls::abi_decode, |call| match call {
            ITempoOracleCalls::currencyCount(_) => {
                metadata::<ITempoOracle::currencyCountCall>(|| self.currency_count())
            }
            ITempoOracleCalls::registeredCurrencyAt(call) => {
                view(call, |c| self.registered_currency_at(c.index))
            }
            ITempoOracleCalls::isCurrencyRegistered(call) => {
                view(call, |c| self.is_currency_registered(c.currencyId))
            }
            ITempoOracleCalls::registerCurrency(call) => {
                mutate_void(call, msg_sender, |s, c| {
                    self.register_currency(s, c.currencyId)
                })
            }
            ITempoOracleCalls::unregisterCurrency(call) => {
                mutate_void(call, msg_sender, |s, c| {
                    self.unregister_currency(s, c.currencyId)
                })
            }
            ITempoOracleCalls::setPrices(call) => {
                mutate_void(call, msg_sender, |s, c| self.set_prices(s, c))
            }
            ITempoOracleCalls::updatePriceFeed(call) => {
                mutate_void(call, msg_sender, |s, c| self.update_price_feed(s, c))
            }
            ITempoOracleCalls::setPriceFeed(call) => {
                mutate_void(call, msg_sender, |s, c| self.set_price_feed(s, c))
            }
            ITempoOracleCalls::getOraclePrice(call) => {
                view(call, |c| self.get_oracle_price(c.currencyId))
            }
            ITempoOracleCalls::getCurrencies(_) => {
                metadata::<ITempoOracle::getCurrenciesCall>(|| self.get_currencies())
            }
            ITempoOracleCalls::getPairPrice(call) => {
                view(call, |c| self.get_pair_price(c.base, c.quote))
            }
            ITempoOracleCalls::oracleMaxDeviationBps(_) => {
                metadata::<ITempoOracle::oracleMaxDeviationBpsCall>(|| {
                    self.read_oracle_max_deviation_bps()
                })
            }
            ITempoOracleCalls::oracleFeedThreshold(_) => {
                metadata::<ITempoOracle::oracleFeedThresholdCall>(|| {
                    self.read_oracle_feed_threshold()
                })
            }
            ITempoOracleCalls::setOracleMaxDeviationBps(call) => {
                mutate_void(call, msg_sender, |s, c| {
                    self.set_oracle_max_deviation_bps(s, c)
                })
            }
            ITempoOracleCalls::setOracleFeedThreshold(call) => {
                mutate_void(call, msg_sender, |s, c| {
                    self.set_oracle_feed_threshold(s, c)
                })
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        storage::{StorageCtx, hashmap::HashMapStorageProvider},
        test_util::{assert_full_coverage, check_selector_coverage},
    };
    use tempo_contracts::precompiles::ITempoOracle::ITempoOracleCalls;

    #[test]
    fn test_tempo_oracle_selector_coverage() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        StorageCtx::enter(&mut storage, || {
            let mut oracle = TempoOracle::new();

            let unsupported = check_selector_coverage(
                &mut oracle,
                ITempoOracleCalls::SELECTORS,
                "ITempoOracle",
                ITempoOracleCalls::name_by_selector,
            );

            assert_full_coverage([unsupported]);
            Ok(())
        })
    }
}
