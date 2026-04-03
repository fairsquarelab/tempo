//! Scan sealed blocks over JSON-RPC and validate TempoOracle top-of-block ordering.

use alloy::{
    consensus::Transaction,
    primitives::Address,
    providers::{Provider, ProviderBuilder},
    rpc::types::BlockNumberOrTag,
    sol_types::SolCall as _,
};
use clap::Parser;
use eyre::{Context as _, bail};
use tempo_contracts::precompiles::{ITempoOracle, TEMPO_ORACLE_ADDRESS};

#[derive(Debug, Parser)]
pub(crate) struct VerifyOracleBlocks {
    /// HTTP RPC URL (e.g. http://127.0.0.1:8545 for localnet node 0).
    #[arg(long, default_value = "http://127.0.0.1:8545")]
    rpc: String,

    #[arg(long, default_value_t = 1)]
    from_block: u64,

    /// Inclusive end block (default: latest sealed block from `eth_blockNumber`).
    #[arg(long)]
    to_block: Option<u64>,

    /// Exit with failure if any block in range has no oracle prefix (criterion B in README).
    #[arg(long, default_value_t = false)]
    require_every_block: bool,
}

impl VerifyOracleBlocks {
    pub(crate) async fn run(self) -> eyre::Result<()> {
        let provider = ProviderBuilder::new()
            .connect(&self.rpc)
            .await
            .wrap_err("failed to connect to RPC")?;

        let latest = provider
            .get_block_number()
            .await
            .wrap_err("eth_blockNumber")?;

        let end = self.to_block.unwrap_or(latest);

        if end < self.from_block {
            bail!("to_block ({end}) < from_block ({})", self.from_block);
        }

        let update_sel = ITempoOracle::updatePriceFeedCall::SELECTOR;
        let set_sel = ITempoOracle::setPriceFeedCall::SELECTOR;

        let mut checked_with_prefix = 0u64;
        let mut empty_ok = 0u64;

        for n in self.from_block..=end {
            let block = provider
                .get_block_by_number(BlockNumberOrTag::Number(n))
                .full()
                .await
                .wrap_err_with(|| format!("eth_getBlockByNumber full {n}"))?
                .ok_or_else(|| eyre::eyre!("block {n} not found"))?;

            let txs = block.into_transactions_vec();
            let oracle_addr: Address = TEMPO_ORACLE_ADDRESS;

            let mut oracle_len = 0usize;
            for tx in &txs {
                if tx.to() == Some(oracle_addr) {
                    oracle_len += 1;
                } else {
                    break;
                }
            }

            if oracle_len == 0 {
                if self.require_every_block {
                    bail!("block {n}: expected oracle prefix (--require-every-block), found none");
                }
                empty_ok += 1;
                continue;
            }

            checked_with_prefix += 1;

            for (i, tx) in txs.iter().take(oracle_len).enumerate() {
                let input = tx.input();
                if input.len() < 4 {
                    bail!("block {n}: oracle tx {i} calldata too short");
                }
                let sel: [u8; 4] = input[0..4].try_into().expect("len checked");
                let is_last = i + 1 == oracle_len;
                if is_last {
                    if sel != set_sel {
                        bail!(
                            "block {n}: last oracle tx must be setPriceFeed, got selector {sel:?}"
                        );
                    }
                } else if sel != update_sel {
                    bail!(
                        "block {n}: oracle tx {i} must be updatePriceFeed before final setPriceFeed, got selector {sel:?}"
                    );
                }
            }
        }

        println!(
            "OK: scanned blocks {}..={} (latest RPC head: {latest}). Blocks with oracle prefix: {checked_with_prefix}; blocks without oracle txs (allowed in default mode): {empty_ok}",
            self.from_block, end
        );
        Ok(())
    }
}
