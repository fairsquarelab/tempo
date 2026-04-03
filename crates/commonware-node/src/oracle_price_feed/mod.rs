//! Actor that routes signed TempoOracle `updatePriceFeed` transactions to the next block proposer
//! over [`crate::config::ORACLE_PRICE_FEED_CHANNEL_IDENT`], analogous to [`crate::subblocks::Actor`].
//!
//! Validated feeds are staged internally and served to the payload builder via
//! [`Mailbox::get_oracle_price_feeds`] (same sync RPC pattern as subblocks `get_subblocks`).
//!
//! HTTP price fetches run on a **dedicated background poller** (see `OracleConfig::poll_interval_ms`)
//! and only update `cached_scalars` via `PricesUpdated` actor messages. Consensus events build
//! `updatePriceFeed` from that cache immediately without waiting on HTTP.
//!
//! ## Flow
//!
//! 1. Background loop periodically fetches prices and refreshes the cache.
//! 2. View N-1 notarizes → View N starts.
//! 3. **Non-leader** validators read the cache, build `updatePriceFeed`, and send to the leader via P2P.
//! 4. **Leader** validates each feed via EVM execution and stages them as top-of-block system txs.
//! 5. Block verification re-checks the oracle transactions included in the proposed block.
//!
//! **Tracing**: `tempo_commonware_node::oracle_price_feed=info` logs leader bundle handoff
//! (`updatePriceFeed` / `setPriceFeed` staging). Use `=debug` for finer steps. Failures are **warn**
//! by default (decode, P2P, validation, mailbox, build).

pub mod config;
pub mod currency;
pub mod fx_price;
pub(crate) mod price_source;
pub(crate) mod sources;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvError;

use crate::{consensus::Digest, epoch::SchemeProvider, subblocks::evm_at_block};
use alloy_consensus::transaction::TxHashRef;
use alloy_consensus::{
    BlockHeader as _, SignableTransaction as _, Signed, Transaction as AlloyTransaction, TxLegacy,
};
use alloy_eips::eip2718::{Decodable2718, Encodable2718};
use alloy_primitives::{Address, B256, BlockHash, Bytes, TxKind};
use alloy_signer::SignerSync;
use alloy_sol_types::SolCall;
use commonware_consensus::{
    Epochable, Reporter, Viewable,
    simplex::{
        elector::Random,
        scheme::bls12381_threshold::vrf::{Certificate, Scheme},
        types::Activity,
    },
    types::{Epocher as _, FixedEpocher, Height, Round, View},
};
use commonware_cryptography::{
    Signer,
    bls12381::primitives::variant::MinSig,
    certificate::Provider,
    ed25519::{PrivateKey, PublicKey},
};
use commonware_p2p::{Receiver, Recipients, Sender};
use commonware_runtime::{IoBuf, Metrics, Spawner};
use eyre::Context as _;
use futures::{StreamExt as _, channel::mpsc};
use reth_evm::Evm as _;
use reth_primitives_traits::{Recovered, SignerRecoverable};
use reth_provider::{BlockReader, BlockSource, StateProviderFactory};
use tempo_contracts::precompiles::{ITempoOracle, TEMPO_ORACLE_ADDRESS};
use tempo_node::TempoFullNode;
use tempo_primitives::TempoTxEnvelope;
use tracing::{Instrument, Level, Span, debug, info, instrument, warn};

use config::OracleConfig;
use currency::{BASE_CURRENCY, Currency};
use fx_price::FxPrice;
use price_source::FalloverFetcher;

/// Declared `gas_limit` for signed oracle legacy transactions.
///
/// Keep this **well below** the T1+ general lane cap (30M): the payload builder rejects the next
/// oracle tx when `non_payment_gas_used + tx.gas_limit` exceeds that cap, even if execution would
/// only burn a small amount of gas.
#[inline]
fn oracle_update_price_feed_gas_limit(currency_count: usize) -> u64 {
    // Calldata is `(currencyId, price)[]` (~2 words per currency + dynamic array overhead).
    // Execution still touches VC2, registry, and pending row writes.
    // Previous values (520k base + 130k/currency) caused OOG for 3 currencies; values tripled.
    const BASE: u64 = 1_600_000;
    const PER_CURRENCY: u64 = 400_000;
    (BASE + currency_count as u64 * PER_CURRENCY).clamp(2_000_000, 10_000_000)
}

/// `setPriceFeed` can touch every active validator's pending row and all currencies; allow more
/// headroom than a single `updatePriceFeed` while staying far under the per-tx cap and general
/// lane budget.
const ORACLE_SET_PRICE_FEED_GAS_LIMIT: u64 = 2_000_000;

/// Maximum age (in milliseconds) of a cached price before it is considered stale and excluded
/// from `updatePriceFeed` tx construction. With a 500ms poll interval, 30s allows ~60 missed
/// ticks before prices go stale — enough headroom for transient API outages.
const MAX_STALE_MS: u64 = 30_000;

pub(crate) struct Config<TContext> {
    pub(crate) context: TContext,
    pub(crate) signer: PrivateKey,
    /// SECP256K1 key for signing oracle EVM transactions (`updatePriceFeed`, `setPriceFeed`).
    pub(crate) evm_signer: Option<alloy_signer_local::PrivateKeySigner>,
    pub(crate) fee_recipient: Address,
    pub(crate) scheme_provider: SchemeProvider,
    pub(crate) node: TempoFullNode,
    pub(crate) epoch_strategy: FixedEpocher,
    /// Optional oracle price source configuration loaded from a TOML file.
    pub(crate) oracle_config: Option<OracleConfig>,
}

pub(crate) struct Actor<TContext> {
    actions_tx: mpsc::UnboundedSender<Message>,
    actions_rx: mpsc::UnboundedReceiver<Message>,
    context: TContext,
    signer: PrivateKey,
    evm_signer: Option<alloy_signer_local::PrivateKeySigner>,
    #[allow(dead_code)]
    fee_recipient: Address,
    scheme_provider: SchemeProvider,
    node: TempoFullNode,
    epoch_strategy: FixedEpocher,
    consensus_tip: Option<(Round, BlockHash, Certificate<MinSig>)>,
    cached_next_proposer: Option<PublicKey>,
    /// Validated feeds ready for the payload builder; consumed by [`Message::GetOraclePriceFeeds`].
    oracle_tx_bundle: Vec<Recovered<TempoTxEnvelope>>,
    /// Last tip for which we built an `updatePriceFeed` tx (dedup guard).
    last_update_tip: Option<BlockHash>,
    /// Registered currency IDs → (price, fetch_timestamp_ms). Updated by the background price
    /// poller. Entries older than [`MAX_STALE_MS`] are considered stale and excluded from
    /// `updatePriceFeed` tx construction.
    ///
    /// TODO: include timestamp in `updatePriceFeed` calldata so validators can reject stale
    /// feeds during verification. Future TEE integration will attest the timestamp to prove
    /// the price was genuinely fetched at the claimed time.
    ///
    /// TODO: when on-chain dynamic currency registration is supported, listen for registration
    /// events and re-read currency IDs to update the poller's currency list at runtime.
    cached_prices: HashMap<u32, (FxPrice, u64)>,
    /// Configured price fetcher (None when no oracle config is provided).
    fetcher: Option<Arc<FalloverFetcher>>,
    /// HTTP poll interval from oracle TOML (`0` treated as 1 ms in the poller).
    poll_interval_ms: u64,
}

impl<TContext: Spawner + Metrics> Actor<TContext> {
    pub(crate) fn new(
        Config {
            context,
            signer,
            evm_signer,
            fee_recipient,
            scheme_provider,
            node,
            epoch_strategy,
            oracle_config,
        }: Config<TContext>,
    ) -> Self {
        let fetcher = oracle_config
            .as_ref()
            .map(|cfg| Arc::new(FalloverFetcher::from_config(cfg)));

        let poll_interval_ms = oracle_config
            .as_ref()
            .map(|c| c.poll_interval_ms)
            .unwrap_or(500);

        let (actions_tx, actions_rx) = mpsc::unbounded();
        Self {
            actions_tx,
            actions_rx,
            context,
            signer,
            evm_signer,
            fee_recipient,
            scheme_provider,
            node,
            epoch_strategy,
            consensus_tip: None,
            cached_next_proposer: None,
            oracle_tx_bundle: Vec::new(),
            last_update_tip: None,
            cached_prices: HashMap::new(),
            fetcher,
            poll_interval_ms,
        }
    }

    pub(crate) fn mailbox(&self) -> Mailbox {
        Mailbox {
            tx: self.actions_tx.clone(),
        }
    }

    pub(crate) async fn run(
        mut self,
        (mut network_tx, mut network_rx): (
            impl Sender<PublicKey = PublicKey>,
            impl Receiver<PublicKey = PublicKey>,
        ),
    ) {
        self.spawn_price_poll_thread();
        loop {
            tokio::select! {
                biased;
                Some(action) = self.actions_rx.next() => {
                    match action {
                        Message::Consensus(activity) => {
                            if let Some(tx) = self.on_consensus_event(*activity) {
                                self.handle_oracle_tx(tx, &network_tx);
                            }
                        }
                        other => self.on_new_message(other),
                    }
                }
                // TODO: add a select arm for on-chain currency registry change events.
                // When detected, call `read_currency_ids_from_chain()` and update the
                // poller's shared currency list (e.g. via `Arc<RwLock<Vec<Currency>>>`).
                recv = network_rx.recv() => {
                    match recv {
                        Ok((sender, message)) => {
                            if let Err(e) =
                                self.on_network_message(sender, message, &mut network_tx).await
                            {
                                warn!(error = %e, "oracle: on_network_message failed");
                            }
                        }
                        Err(e) => {
                            warn!(?e, "oracle: P2P recv closed or failed");
                        }
                    }
                }
            }
        }
    }

    /// Spawns a long-lived thread with a single-thread Tokio runtime for periodic HTTP fetches.
    /// Uses `fetcher.take()` so that subsequent calls are no-ops (poller starts at most once).
    fn spawn_price_poll_thread(&mut self) {
        let Some(fetcher) = self.fetcher.take() else {
            return;
        };
        if self.evm_signer.is_none() {
            return;
        }

        // Read registered currency IDs from chain state and resolve to Currency enums.
        let Some(currency_ids) = self.read_currency_ids_from_chain() else {
            warn!("oracle: failed to read currency IDs from chain — poller not started");
            self.fetcher = Some(fetcher); // put it back so we can retry
            return;
        };
        let currencies: Vec<Currency> = currency_ids
            .iter()
            .filter_map(|&id| Currency::try_from(id).ok())
            .filter(|c| !c.is_base())
            .collect();

        if currencies.is_empty() {
            debug!("oracle: no non-base currencies to poll — skipping price poller");
            return;
        }

        let interval_ms = self.poll_interval_ms.max(1);
        let actions_tx = self.actions_tx.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("tempo-oracle-price-poll".into())
            .spawn(move || {
                let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    warn!("oracle: failed to build tokio runtime for price poller");
                    return;
                };
                let in_flight = Arc::new(AtomicBool::new(false));
                rt.block_on(async move {
                    let mut interval =
                        tokio::time::interval(std::time::Duration::from_millis(interval_ms));
                    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                    loop {
                        interval.tick().await;
                        if in_flight
                            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                            .is_err()
                        {
                            continue;
                        }
                        let prices = fetcher.fetch_prices(&currencies).await;
                        // Convert Currency keys to u32 currencyId keys.
                        let scalars: HashMap<u32, FxPrice> = prices
                            .into_iter()
                            .map(|(c, s)| (c.iso_numeric(), s))
                            .collect();
                        if let Err(e) = actions_tx.unbounded_send(Message::PricesUpdated(scalars)) {
                            warn!(
                                ?e,
                                "oracle: PricesUpdated send failed (actor mailbox closed?)"
                            );
                        }
                        in_flight.store(false, Ordering::Release);
                    }
                });
            })
        {
            warn!(?e, "oracle: failed to spawn price poller thread");
        }
    }

    fn on_new_message(&mut self, action: Message) {
        match action {
            Message::Consensus(_) => unreachable!("Consensus handled in event loop"),
            Message::GetOraclePriceFeeds { response } => {
                let mut bundle = std::mem::take(&mut self.oracle_tx_bundle);
                if bundle.is_empty() {
                    debug!("oracle: get_oracle_price_feeds (staged empty)");
                } else {
                    let update_hashes: Vec<String> = bundle
                        .iter()
                        .map(|tx| format!("{}", tx.inner().tx_hash()))
                        .collect();
                    let update_txs_from_evm_signer = self
                        .evm_signer
                        .as_ref()
                        .map(|evm| {
                            bundle
                                .iter()
                                .filter(|tx| tx.recover_signer().ok() == Some(evm.address()))
                                .count() as u64
                        })
                        .unwrap_or(0);
                    let update_count = bundle.len();
                    let mut appended_set_price_feed = false;
                    if let Some(set_tx) = self.build_set_price_feed_tx(update_txs_from_evm_signer) {
                        let set_hash = *set_tx.inner().tx_hash();
                        bundle.push(set_tx);
                        appended_set_price_feed = true;
                        info!(
                            update_txs = update_count,
                            total_in_bundle = bundle.len(),
                            %set_hash,
                            update_hashes = %update_hashes.join(","),
                            "oracle: handing bundle to payload builder (updatePriceFeed txs + setPriceFeed)"
                        );
                    } else {
                        warn!(
                            update_txs = update_count,
                            update_txs_from_evm_signer,
                            update_hashes = %update_hashes.join(","),
                            "oracle: build_set_price_feed_tx failed; returning updates without setPriceFeed (payload may revert)"
                        );
                    }
                    debug!(
                        update_txs = update_count,
                        appended_set_price_feed,
                        total_txs = bundle.len(),
                        update_txs_from_evm_signer,
                        "oracle: get_oracle_price_feeds -> payload builder"
                    );
                }
                if response.send(bundle).is_err() {
                    warn!("oracle: get_oracle_price_feeds response channel closed before send");
                }
            }
            Message::ValidatedFeed(tx) => {
                self.oracle_tx_bundle.push(tx);
            }
            Message::PricesUpdated(scalars) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                for (id, price) in scalars {
                    self.cached_prices.insert(id, (price, now));
                }
            }
        }
    }

    fn tip(&self) -> Option<BlockHash> {
        self.consensus_tip.as_ref().map(|(_, tip, _)| *tip)
    }

    /// Legacy `gas_price` must be ≥ parent base fee or validation returns `GasPriceLessThanBasefee`.
    fn legacy_gas_price_at_tip(&self, tip: BlockHash) -> u128 {
        self.node
            .provider
            .find_block_by_hash(tip, BlockSource::Any)
            .ok()
            .flatten()
            .and_then(|h| h.base_fee_per_gas())
            .map(u128::from)
            .unwrap_or(1)
            .max(1)
    }

    fn scheme_for_child_of_tip(&self) -> Option<Arc<Scheme<PublicKey, MinSig>>> {
        let tip = self.tip()?;
        let header = self
            .node
            .provider
            .find_block_by_hash(tip, BlockSource::Any)
            .ok()??;
        let epoch_of_next_block = self
            .epoch_strategy
            .containing(Height::new(header.number() + 1))
            .expect("epoch strategy covers all epochs")
            .epoch();
        self.scheme_provider.scoped(epoch_of_next_block)
    }

    /// Reads all registered currency IDs from the latest on-chain state in registration order.
    /// Uses `provider.latest()` so it works even before a consensus tip is available (genesis).
    fn read_currency_ids_from_chain(&self) -> Option<Vec<u32>> {
        use alloy_primitives::keccak256 as akeccak;
        use reth_provider::{StateProvider, StateProviderFactory};
        use tempo_precompiles::{
            storage::{StorableType as _, packing::extract_from_word},
            tempo_oracle::slots as oracle_slots,
        };

        /// Sanity cap: garbage `currency_count` from mis-decoded storage must not allocate huge vectors.
        const MAX_ORACLE_CURRENCIES: u64 = 256;

        let state = self.node.provider.latest().ok()?;

        // `currency_count` is packed after `registry_admin` in the same EVM word; use slot + byte offset.
        let currency_count_slot_word = state
            .storage(TEMPO_ORACLE_ADDRESS, oracle_slots::CURRENCY_COUNT.into())
            .ok()??;
        let currency_count: u64 = extract_from_word(
            currency_count_slot_word,
            oracle_slots::CURRENCY_COUNT_OFFSET,
            u64::BYTES,
        )
        .ok()?;
        if currency_count > MAX_ORACLE_CURRENCIES {
            return None;
        }

        let base = oracle_slots::CURRENCY_IDS;
        let mut currency_ids = Vec::with_capacity(currency_count as usize);

        for i in 0u64..currency_count {
            // Mapping<u64, u32>: slot = keccak256(lpad(i_be8, 32) ++ base_be32)
            let mut buf = [0u8; 64];
            buf[24..32].copy_from_slice(&i.to_be_bytes());
            buf[32..64].copy_from_slice(&base.to_be_bytes::<32>());
            let slot = alloy_primitives::U256::from_be_bytes(akeccak(buf).0);

            let word = state
                .storage(TEMPO_ORACLE_ADDRESS, slot.into())
                .ok()
                .flatten()?;
            // u32 is stored right-aligned in a 32-byte EVM word.
            let cid: u32 = extract_from_word(word, 0, u32::BYTES).ok()?;
            currency_ids.push(cid);
        }

        Some(currency_ids)
    }

    /// Builds a signed `updatePriceFeed(OracleCurrencyPrice[] updates)` transaction using
    /// cached prices. Excludes the base currency (USD) and currencies without a cached price.
    /// Returns `None` if no currencies have prices or EVM signer is missing.
    fn build_update_price_feed_tx(&mut self) -> Option<Recovered<TempoTxEnvelope>> {
        use reth_ethereum::chainspec::EthChainSpec;

        let evm_signer = self.evm_signer.as_ref()?;
        let tip = self.tip()?;

        // Build updates: exclude base currency, zero prices, and stale entries.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let updates: Vec<ITempoOracle::OracleCurrencyPrice> = self
            .cached_prices
            .iter()
            .filter(|&(&id, _)| id != BASE_CURRENCY.iso_numeric())
            .filter(|&(_, (price, ts))| !price.is_zero() && now.saturating_sub(*ts) <= MAX_STALE_MS)
            .map(|(&id, (price, _))| ITempoOracle::OracleCurrencyPrice {
                currencyId: id,
                price: price.inner(),
            })
            .collect();

        if updates.is_empty() {
            return None;
        }

        let calldata = ITempoOracle::updatePriceFeedCall {
            updates: updates.clone(),
        }
        .abi_encode();

        let nonce = self
            .node
            .provider
            .state_by_block_hash(tip)
            .ok()
            .and_then(|state| state.basic_account(&evm_signer.address()).ok().flatten())
            .map(|acc| acc.nonce)
            .unwrap_or(0);

        let gas_price = self.legacy_gas_price_at_tip(tip);
        let tx = TxLegacy {
            chain_id: Some(self.node.chain_spec().chain().id()),
            nonce,
            gas_price,
            gas_limit: oracle_update_price_feed_gas_limit(updates.len()),
            to: TxKind::Call(TEMPO_ORACLE_ADDRESS),
            value: alloy_primitives::U256::ZERO,
            input: Bytes::from(calldata),
        };

        let sig = match evm_signer.sign_hash_sync(&tx.signature_hash()) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?e,
                    "oracle: build_update_price_feed_tx failed — sign_hash_sync error"
                );
                return None;
            }
        };
        let envelope = TempoTxEnvelope::Legacy(Signed::new_unhashed(tx, sig.into()));
        Some(
            envelope
                .try_into_recovered()
                .expect("just signed; recovery must succeed"),
        )
    }

    /// Builds a signed `setPriceFeed()` transaction to close the oracle bundle.
    ///
    /// `update_txs_from_evm_signer` is the number of `updatePriceFeed` txs in the bundle signed by
    /// this node's EVM oracle key (must be added to the on-chain nonce before `setPriceFeed`).
    ///
    /// The recovered signer must equal the block beneficiary (`consensus.fee-recipient`); the
    /// execution layer rejects the tx otherwise (`tempo-evm` `validate_set_price_feed_tx`).
    fn build_set_price_feed_tx(
        &self,
        update_txs_from_evm_signer: u64,
    ) -> Option<Recovered<TempoTxEnvelope>> {
        use reth_ethereum::chainspec::EthChainSpec;

        let evm_signer = self.evm_signer.as_ref()?;
        let tip = self.tip()?;

        let nonce = self
            .node
            .provider
            .state_by_block_hash(tip)
            .ok()
            .and_then(|state| state.basic_account(&evm_signer.address()).ok().flatten())
            .map(|acc| acc.nonce)
            .unwrap_or(0);

        let gas_price = self.legacy_gas_price_at_tip(tip);
        let tx = TxLegacy {
            chain_id: Some(self.node.chain_spec().chain().id()),
            nonce: nonce + update_txs_from_evm_signer,
            gas_price,
            gas_limit: ORACLE_SET_PRICE_FEED_GAS_LIMIT,
            to: TxKind::Call(TEMPO_ORACLE_ADDRESS),
            value: alloy_primitives::U256::ZERO,
            input: Bytes::from(ITempoOracle::setPriceFeedCall::SELECTOR.as_slice()),
        };

        let sig = match evm_signer.sign_hash_sync(&tx.signature_hash()) {
            Ok(s) => s,
            Err(e) => {
                warn!(
                    ?e,
                    "oracle: build_set_price_feed_tx failed — sign_hash_sync error"
                );
                return None;
            }
        };
        let envelope = TempoTxEnvelope::Legacy(Signed::new_unhashed(tx, sig.into()));
        Some(
            envelope
                .try_into_recovered()
                .expect("just signed; recovery must succeed"),
        )
    }

    #[instrument(skip_all, fields(event.epoch = %event.epoch(), event.view = %event.view()))]
    fn on_consensus_event(
        &mut self,
        event: Activity<Scheme<PublicKey, MinSig>, Digest>,
    ) -> Option<Recovered<TempoTxEnvelope>> {
        let (new_tip, new_round, new_cert) = match event {
            Activity::Notarization(n) => {
                (Some(n.proposal.payload.0), n.proposal.round, n.certificate)
            }
            Activity::Finalization(n) => {
                (Some(n.proposal.payload.0), n.proposal.round, n.certificate)
            }
            Activity::Nullification(n) => (None, n.round, n.certificate),
            _ => return None,
        };

        if let Some((round, tip, cert)) = &mut self.consensus_tip
            && *round <= new_round
        {
            *round = new_round;
            *cert = new_cert;
            if let Some(new_tip) = new_tip
                && *tip != new_tip
            {
                self.oracle_tx_bundle.clear();
                *tip = new_tip;
            }
        } else if self.consensus_tip.is_none()
            && let Some(new_tip) = new_tip
        {
            self.consensus_tip = Some((new_round, new_tip, new_cert));
        }

        let Some((round, tip, certificate)) = &self.consensus_tip else {
            return None;
        };

        let Ok(Some(header)) = self
            .node
            .provider
            .find_block_by_hash(*tip, BlockSource::Any)
        else {
            debug!(?tip, "oracle feed: missing header for tip block");
            return None;
        };

        let epoch_of_next_block = self
            .epoch_strategy
            .containing(Height::new(header.number() + 1))
            .expect("epoch strategy covers all epochs")
            .epoch();

        let Some(scheme) = self.scheme_provider.scoped(epoch_of_next_block) else {
            debug!(%epoch_of_next_block, "oracle feed: scheme not found for epoch");
            return None;
        };

        let next_round = if round.epoch() == epoch_of_next_block {
            Round::new(round.epoch(), round.view().next())
        } else {
            Round::new(epoch_of_next_block, View::new(1))
        };

        let next_proposer = Random::select_leader::<MinSig>(
            next_round,
            scheme.participants().len() as u32,
            certificate.get().map(|signature| signature.seed_signature),
        );
        let next_proposer = scheme.participants()[next_proposer.get() as usize].clone();
        self.cached_next_proposer = Some(next_proposer.clone());

        // Prices come from the background poller (`spawn_price_poll_thread`); this path only reads
        // `cached_prices` and signs — no HTTP on the consensus hot path.
        if self.evm_signer.is_some() && self.tip() != self.last_update_tip {
            if let Some(tx) = self.build_update_price_feed_tx() {
                self.last_update_tip = self.tip();
                return Some(tx);
            } else {
                warn!(
                    tip = ?self.tip(),
                    cached_prices_len = self.cached_prices.len(),
                    "Error building `updatePriceFeed` tx"
                );
            }
        }
        None
    }

    #[instrument(skip_all, err(level = Level::WARN), fields(sender = %sender, msg_bytes = message.len()))]
    async fn on_network_message(
        &mut self,
        sender: PublicKey,
        message: IoBuf,
        network_tx: &mut impl Sender<PublicKey = PublicKey>,
    ) -> eyre::Result<()> {
        let msg = OracleMessage::decode(message.as_ref())
            .wrap_err("failed to decode oracle network message")?;

        match msg {
            OracleMessage::Feed(envelope) => {
                let Some(tip) = self.tip() else {
                    return Err(eyre::eyre!("missing tip of the chain"));
                };

                let Some(scheme) = self.scheme_for_child_of_tip() else {
                    warn!("oracle: rejected Feed — no scheme for child of tip");
                    return Ok(());
                };

                if !scheme.participants().iter().any(|p| *p == sender) {
                    return Err(eyre::eyre!("sender {sender} not in validator set"));
                }

                // Send ack before spawning validation.
                let tx_hash = envelope.tx_hash();
                if let Err(e) = network_tx
                    .send(
                        Recipients::One(sender.clone()),
                        OracleMessage::Ack(*tx_hash).encode(),
                        true,
                    )
                    .await
                {
                    warn!(
                        ?e,
                        %tx_hash,
                        %sender,
                        "oracle: failed to send Ack for received Feed"
                    );
                }

                // Spawn validation as a separate task (subblocks pattern).
                let actions_tx = self.actions_tx.clone();
                let node = self.node.clone();
                let span = Span::current();
                self.context
                    .clone()
                    .with_label("validate_oracle_feed")
                    .shared(true)
                    .spawn(move |_| {
                        validate_oracle_feed(sender, envelope, node, tip, scheme, actions_tx)
                            .instrument(span)
                    });
            }
            OracleMessage::Ack(hash) => {
                debug!(%hash, "received oracle feed ack");
            }
        }

        Ok(())
    }

    fn is_next_proposer(&self) -> bool {
        self.cached_next_proposer
            .as_ref()
            .is_some_and(|p| *p == self.signer.public_key())
    }

    fn handle_oracle_tx(
        &mut self,
        tx: Recovered<TempoTxEnvelope>,
        network_tx: &impl Sender<PublicKey = PublicKey>,
    ) {
        if self.is_next_proposer() {
            self.oracle_tx_bundle.push(tx);
        } else if let Some(proposer) = self.cached_next_proposer.clone() {
            let encoded = OracleMessage::Feed(tx.inner().clone()).encode();
            let mut ntx = network_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = ntx.send(Recipients::One(proposer), encoded, true).await {
                    warn!(?e, "failed to send oracle price feed to next proposer");
                }
            });
        } else {
            warn!("oracle: no cached next proposer, dropping feed");
        }
    }
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

enum Message {
    Consensus(Box<Activity<Scheme<PublicKey, MinSig>, Digest>>),
    GetOraclePriceFeeds {
        response: std::sync::mpsc::SyncSender<Vec<Recovered<TempoTxEnvelope>>>,
    },
    /// A feed that passed validation in a spawned task.
    ValidatedFeed(Recovered<TempoTxEnvelope>),
    /// Fresh scalars returned by the async price fetcher task, keyed by `currencyId`.
    PricesUpdated(HashMap<u32, FxPrice>),
}

impl std::fmt::Debug for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Consensus(_) => f.write_str("Consensus(..)"),
            Self::GetOraclePriceFeeds { .. } => f.write_str("GetOraclePriceFeeds { .. }"),
            Self::ValidatedFeed(_) => f.write_str("ValidatedFeed(..)"),
            Self::PricesUpdated(_) => f.write_str("PricesUpdated(..)"),
        }
    }
}

// ---------------------------------------------------------------------------
// Network message envelope
// ---------------------------------------------------------------------------

/// Wire messages exchanged over the oracle price feed P2P channel.
#[derive(Debug)]
enum OracleMessage {
    /// A signed `updatePriceFeed` transaction sent from a validator to the leader.
    Feed(TempoTxEnvelope),
    /// Acknowledgement that the leader received and accepted the feed.
    Ack(B256),
}

impl OracleMessage {
    fn encode(self) -> bytes::Bytes {
        match self {
            Self::Feed(tx) => {
                let mut buf = Vec::with_capacity(1 + tx.encode_2718_len());
                buf.push(0x00);
                tx.encode_2718(&mut buf);
                buf.into()
            }
            Self::Ack(hash) => {
                let mut buf = Vec::with_capacity(1 + 32);
                buf.push(0x01);
                buf.extend_from_slice(hash.as_ref());
                buf.into()
            }
        }
    }

    fn decode(message: &[u8]) -> eyre::Result<Self> {
        eyre::ensure!(!message.is_empty(), "empty oracle message");
        match message[0] {
            0x00 => {
                let tx = TempoTxEnvelope::decode_2718_exact(&message[1..])
                    .map_err(|e| eyre::eyre!("oracle feed decode_2718: {e}"))?;
                Ok(Self::Feed(tx))
            }
            0x01 => {
                eyre::ensure!(message.len() == 33, "invalid ack length");
                Ok(Self::Ack(B256::from_slice(&message[1..])))
            }
            tag => Err(eyre::eyre!("unknown oracle message tag: {tag}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Mailbox
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct Mailbox {
    tx: mpsc::UnboundedSender<Message>,
}

impl Mailbox {
    /// Returns staged oracle txs for the payload builder (same sync RPC pattern as [`crate::subblocks::Mailbox::get_subblocks`]).
    pub(crate) fn get_oracle_price_feeds(
        &self,
    ) -> Result<Vec<Recovered<TempoTxEnvelope>>, RecvError> {
        let (response_tx, response_rx) = std::sync::mpsc::sync_channel(1);
        if self
            .tx
            .unbounded_send(Message::GetOraclePriceFeeds {
                response: response_tx,
            })
            .is_err()
        {
            warn!("oracle: get_oracle_price_feeds request dropped (actor mailbox closed)");
            return Err(RecvError);
        }
        response_rx.recv()
    }
}

impl Reporter for Mailbox {
    type Activity = Activity<Scheme<PublicKey, MinSig>, Digest>;

    async fn report(&mut self, activity: Self::Activity) {
        if self
            .tx
            .unbounded_send(Message::Consensus(Box::new(activity)))
            .is_err()
        {
            warn!("oracle: consensus event send failed (actor mailbox closed)");
        }
    }
}

// ---------------------------------------------------------------------------
// Validation (spawned task)
// ---------------------------------------------------------------------------

/// Validates a single oracle price feed transaction.
///
/// Checks:
/// 1. Signer recovery
/// 2. Transaction targets the TempoOracle `updatePriceFeed` selector
/// 3. Sender is a member of the current validator set
/// 4. EVM execution succeeds against the tip state
#[instrument(skip_all, fields(sender = %sender))]
async fn validate_oracle_feed(
    sender: PublicKey,
    envelope: TempoTxEnvelope,
    node: TempoFullNode,
    tip: BlockHash,
    scheme: Arc<Scheme<PublicKey, MinSig>>,
    actions_tx: mpsc::UnboundedSender<Message>,
) -> eyre::Result<()> {
    let tx = envelope
        .try_into_recovered()
        .map_err(|_| eyre::eyre!("oracle feed signer recovery failed"))?;

    if !is_update_price_feed_tx(&tx) {
        return Err(eyre::eyre!("not a valid updatePriceFeed tx"));
    }

    if !scheme.participants().iter().any(|p| *p == sender) {
        return Err(eyre::eyre!("sender {sender} not in validator set"));
    }

    // EVM execution validation (same pattern as subblocks::validate_subblock).
    let mut evm =
        evm_at_block(&node, tip).map_err(|e| e.wrap_err("failed to create EVM at tip"))?;
    if let Err(err) = evm.transact_commit(&tx) {
        return Err(eyre::eyre!("oracle feed tx execution failed: {err:?}"));
    }

    if actions_tx
        .unbounded_send(Message::ValidatedFeed(tx))
        .is_err()
    {
        warn!("oracle: ValidatedFeed send failed — oracle actor mailbox closed");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_update_price_feed_tx(tx: &Recovered<TempoTxEnvelope>) -> bool {
    use AlloyTransaction as _;
    tx.to() == Some(TEMPO_ORACLE_ADDRESS)
        && tx.input().len() >= 4
        && tx.input()[0..4] == ITempoOracle::updatePriceFeedCall::SELECTOR
}
