use bitcoin::{hashes::Hash, Txid};
use strata_db_types::{
    common::L1TxId,
    l1_broadcast::{L1TxEntry, L1TxStatus},
};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::{
    error::{BroadcasterError, BroadcasterResult},
    io::{BroadcasterIoContext, PublishTxOutcome, TxConfirmationInfo, TxLookupOutcome},
    state::{BroadcasterState, IndexedEntry},
};
use crate::BtcioParams;

/// Processes unfinalized entries and returns the indexed entries whose status changed.
pub(super) async fn process_unfinalized_entries<C>(
    unfinalized_entries: impl Iterator<Item = &IndexedEntry>,
    io: &C,
    params: &BtcioParams,
) -> BroadcasterResult<Vec<IndexedEntry>>
where
    C: BroadcasterIoContext,
{
    let mut updated_entries = Vec::new();

    for entry in unfinalized_entries {
        let idx = *entry.index();
        let txentry = entry.item();
        let txid = txentry
            .try_to_tx()
            .map_err(|e| BroadcasterError::Other(e.to_string()))?
            .compute_txid();

        let updated_status = process_tx_entry(io, txentry, &txid, params).await?;

        if let Some(status) = updated_status {
            let mut new_txentry = txentry.clone();
            new_txentry.status = status;
            updated_entries.push(IndexedEntry::new(idx, new_txentry));
        }
    }

    Ok(updated_entries)
}

/// Computes the next status for a single entry, or `None` when no update is needed.
#[instrument(
    level = "debug",
    skip_all,
    fields(component = "btcio_broadcaster", %txid),
    name = "process_txentry"
)]
pub(super) async fn process_tx_entry<C>(
    io: &C,
    txentry: &L1TxEntry,
    txid: &Txid,
    params: &BtcioParams,
) -> BroadcasterResult<Option<L1TxStatus>>
where
    C: BroadcasterIoContext,
{
    let result = match txentry.status {
        L1TxStatus::Unpublished => publish_tx(io, params, txentry).await.map(Some),
        L1TxStatus::Published => probe_published_entry(io, txentry, txid, params)
            .await
            .map(Some),
        L1TxStatus::Confirmed { .. } => check_tx_confirmations(io, txentry, txid, params)
            .await
            .map(Some),
        L1TxStatus::Finalized { .. } => Ok(None),
        L1TxStatus::InvalidInputs | L1TxStatus::Replaced { .. } => Ok(None),
    };
    if let Ok(ref updated_status) = result {
        debug!(?updated_status);
    }
    result
}

/// Resolves a `Published` entry from a confirmation lookup, falling back to a
/// re-publish probe only when the lookup actually misses.
///
/// `gettransaction` returns three distinguishable shapes for a Published entry:
///
/// 1. `Some(info)` with `confirmations >= 1`: tx mined, derive Confirmed/Finalized.
/// 2. `Some(info)` with `confirmations == 0`: tx alive in mempool. Hold at Published. Re-publishing
///    here would only spam bitcoind and the logs every poll for the entire pre-confirmation window.
/// 3. `Ok(None)`: not found at all. Could be a transient wallet-syncer miss for a freshly broadcast
///    tx, or a genuinely dropped tx (mempool eviction, RBF). Re-publish to disambiguate: benign
///    mempool messages fold back to Published, `bad-txns-inputs-missingorspent` routes to
///    InvalidInputs so the watcher rebuilds the envelope, unless an ancestor of this entry's own
///    replacement chain took those inputs by confirming.
async fn probe_published_entry<C>(
    io: &C,
    txentry: &L1TxEntry,
    txid: &Txid,
    params: &BtcioParams,
) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    let txinfo_res = io.get_transaction(txid).await;
    debug!(?txinfo_res, "checked transaction status");
    let reorg_safe_depth: i64 = params.l1_reorg_safe_depth().into();

    match txinfo_res? {
        // `confirmation_status` returns `Published` for 0-conf, which is the
        // correct sighting for an already-Published entry still in mempool.
        TxLookupOutcome::Found(info) => {
            // Bitcoin Core reports negative confirmations for a wallet transaction that conflicts
            // with one already in the best chain. For an ordinary entry that reads as "side branch
            // after a reorg" and is held at `Published` so it can be re-mined. For an entry that is
            // itself a replacement it is unambiguous: some other transaction in its replacement
            // chain confirmed, so this one can never enter the chain. Holding it at `Published`
            // would leave the envelope waiting on a commit that will never confirm.
            //
            // The predicate is the reverse `replaces` link, not the presence of RBF metadata:
            // every writer-owned entry carries that metadata from its first attempt, and a first
            // attempt has no chain to lose to.
            if txentry.rbf.and_then(|rbf| rbf.replaces).is_some() && info.confirmations < 0 {
                warn!(%txid, confirmations = info.confirmations, "replacement-chain transaction conflicts with a confirmed transaction");
                return resolve_invalid_inputs(io, params, txid, txentry).await;
            }
            Ok(confirmation_status(&info, reorg_safe_depth))
        }
        // A rejected re-publish resolves the same way a first publish does, including the
        // ancestor check, so a `Replaced` verdict must reach the caller rather than fold into
        // `Published`.
        TxLookupOutcome::Missing => match publish_tx(io, params, txentry).await? {
            status @ (L1TxStatus::InvalidInputs | L1TxStatus::Replaced { .. }) => Ok(status),
            _ => Ok(L1TxStatus::Published),
        },
        TxLookupOutcome::RetryLater { reason } => {
            warn!(%reason, "transaction lookup should be retried on next poll");
            Ok(L1TxStatus::Published)
        }
    }
}

/// Bound on how far back a conflicting-ancestor search walks, matching the forward chain walk.
const MAX_ANCESTOR_SEARCH_HOPS: usize = 32;

/// Settles an `InvalidInputs` verdict against the entry's own replacement chain.
///
/// `InvalidInputs` is what sends the writer off to rebuild, and rebuilding republishes a payload a
/// confirmed transaction may already carry. For an entry in a replacement chain the transaction
/// that took its inputs is usually an ancestor of that very chain: a miner included an original
/// after the local node had already accepted its replacement.
///
/// Returns [`L1TxStatus::Replaced`] pointing at the ancestor that won. When no ancestor did, the
/// conflict is with a transaction outside this chain and the [`L1TxStatus::InvalidInputs`] verdict
/// stands. An entry with no `replaces` link, which is every entry outside a replacement chain,
/// resolves to `InvalidInputs` without asking bitcoind anything.
async fn resolve_invalid_inputs<C>(
    io: &C,
    params: &BtcioParams,
    txid: &Txid,
    txentry: &L1TxEntry,
) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    if let Some(winner) = adopt_confirmed_ancestor(io, params, txid, txentry).await? {
        return Ok(L1TxStatus::Replaced { by: winner });
    }
    Ok(L1TxStatus::InvalidInputs)
}

/// Repoints a replacement chain at a superseded ancestor that was mined anyway.
///
/// Bitcoin Core reports negative confirmations for a transaction conflicting with one already in
/// the best chain. For a replacement chain the conflict is usually its own ancestor: the local node
/// accepted the replacement and evicted the original, then a miner included the original from
/// somewhere else. The ancestor is still marked [`L1TxStatus::Replaced`], so the live-entry lookup
/// walks straight past it and every consumer concludes the chain is dead.
///
/// Walks `replaces` back-pointers, asks bitcoind about each ancestor, and on finding a confirmed
/// one reverses the link so the chain resolves to the winner. Returns the winner's txid when the
/// reversal applied, or `None` when no ancestor won, which means the conflict is with a transaction
/// outside this chain and the caller's `InvalidInputs` verdict stands.
async fn adopt_confirmed_ancestor<C>(
    io: &C,
    params: &BtcioParams,
    loser_txid: &Txid,
    loser_entry: &L1TxEntry,
) -> BroadcasterResult<Option<L1TxId>>
where
    C: BroadcasterIoContext,
{
    let reorg_safe_depth: i64 = params.l1_reorg_safe_depth().into();
    let mut ancestor = loser_entry.rbf.and_then(|rbf| rbf.replaces);

    for _ in 0..MAX_ANCESTOR_SEARCH_HOPS {
        let Some(candidate) = ancestor else {
            return Ok(None);
        };
        let candidate_raw = Buf32(candidate.0);
        let Some(candidate_entry) = io.get_tx_entry_by_id(candidate_raw).await? else {
            return Ok(None);
        };

        let candidate_txid = candidate_entry
            .try_to_tx()
            .map_err(|err| BroadcasterError::Other(err.to_string()))?
            .compute_txid();
        if let TxLookupOutcome::Found(info) = io.get_transaction(&candidate_txid).await? {
            if info.confirmations > 0 {
                let winner_status = confirmation_status(&info, reorg_safe_depth);
                info!(
                    loser = %loser_txid,
                    winner = %candidate_txid,
                    confirmations = info.confirmations,
                    "superseded transaction won on-chain; adopting it instead of rebuilding"
                );
                // `false` means another poll already reversed this pair, or the chain no longer
                // links the two. Either way the caller must not report the loser as invalid.
                if io
                    .adopt_confirmed_ancestor(
                        Buf32(loser_txid.to_byte_array()),
                        candidate_raw,
                        winner_status,
                    )
                    .await?
                {
                    return Ok(Some(candidate));
                }
                return Ok(None);
            }
        }

        ancestor = candidate_entry.rbf.and_then(|rbf| rbf.replaces);
    }

    warn!(loser = %loser_txid, "conflicting-ancestor search exceeded its hop budget");
    Ok(None)
}

/// Maps `TxConfirmationInfo` to the natural confirmation-derived status.
///
/// `confirmations <= 0` means the tx is visible to bitcoind but not anchored
/// to the canonical chain (mempool-only, or on a side branch after reorg);
/// returns [`L1TxStatus::Published`]. Callers that need different 0-conf
/// semantics — e.g. regressing a `Confirmed` entry to `Unpublished` on
/// reorg drop — must override the result themselves.
fn confirmation_status(info: &TxConfirmationInfo, reorg_safe_depth: i64) -> L1TxStatus {
    if info.confirmations <= 0 {
        return L1TxStatus::Published;
    }
    let block_hash = info.block_hash.expect("confirmed tx must have block_hash");
    let block_height = info
        .block_height
        .expect("confirmed tx must have block_height");
    let confirmations = info.confirmations as u64;
    if info.confirmations >= reorg_safe_depth {
        L1TxStatus::Finalized {
            confirmations,
            block_hash,
            block_height,
        }
    } else {
        L1TxStatus::Confirmed {
            confirmations,
            block_hash,
            block_height,
        }
    }
}

/// Resolves a `Confirmed` entry to its next confirmation-derived status. A
/// confirmed tx that disappears or drops to 0 confirmations regresses to
/// `Unpublished` so the broadcaster re-publishes (typical reorg recovery).
///
/// Callers in `Published` state must use `probe_published_entry` instead; that
/// path holds 0-conf and not-found differently to avoid publish/revert
/// oscillation and unnecessary re-broadcasts.
async fn check_tx_confirmations<C>(
    io: &C,
    txentry: &L1TxEntry,
    txid: &Txid,
    params: &BtcioParams,
) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    async {
        let txinfo_res = io.get_transaction(txid).await;
        debug!(?txinfo_res, "checked transaction status");
        let reorg_safe_depth: i64 = params.l1_reorg_safe_depth().into();

        match txinfo_res? {
            TxLookupOutcome::Found(info) if info.confirmations == 0 => Ok(L1TxStatus::Unpublished),
            TxLookupOutcome::Found(info) => Ok(confirmation_status(&info, reorg_safe_depth)),
            TxLookupOutcome::Missing => Ok(L1TxStatus::Unpublished),
            TxLookupOutcome::RetryLater { reason } => {
                warn!(%reason, "transaction lookup should be retried on next poll");
                Ok(txentry.status.clone())
            }
        }
    }
    .instrument(debug_span!(
        "check_tx_confirmations",
        component = "btcio_broadcaster",
        %txid,
        current_status = ?txentry.status
    ))
    .await
}

/// Attempts to broadcast an unpublished entry and maps publication outcomes to statuses.
///
/// A rejection for spent inputs is settled against the replacement chain before it becomes
/// [`L1TxStatus::InvalidInputs`]: an original mined between the replacement being persisted and
/// broadcast takes the inputs out from under it, and the replacement is then rejected on its very
/// first send, without ever reaching `Published` where the negative-confirmation path would have
/// caught the same conflict.
async fn publish_tx<C>(
    io: &C,
    params: &BtcioParams,
    txentry: &L1TxEntry,
) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    let tx = txentry.try_to_tx().expect("could not deserialize tx");
    let txid = tx.compute_txid();
    let input_count = tx.input.len();
    let output_count = tx.output.len();

    async {
        if tx.input.is_empty() {
            error!("tx has no inputs, excluding from broadcast");
            return Ok(L1TxStatus::InvalidInputs);
        }

        debug!("publishing tx");
        match io.send_raw_transaction(&tx).await {
            Ok(PublishTxOutcome::Published) => Ok(L1TxStatus::Published),
            Ok(PublishTxOutcome::AlreadyInMempool) => Ok(L1TxStatus::Published),
            Ok(PublishTxOutcome::InvalidInputs) => {
                warn!("tx excluded due to invalid inputs");
                resolve_invalid_inputs(io, params, &txid, txentry).await
            }
            Ok(PublishTxOutcome::RetryLater { reason }) => {
                warn!(%reason, "broadcast should be retried on next poll");
                Ok(L1TxStatus::Unpublished)
            }
            Err(err) => {
                warn!(%err, "errored while broadcasting");
                Err(err)
            }
        }
    }
    .instrument(debug_span!(
        "publish_tx",
        component = "btcio_broadcaster",
        %txid,
        input_count,
        output_count,
        current_status = ?txentry.status
    ))
    .await
}

/// Updates state by folding updated entries and newly seen unfinalized entries from IO context.
pub(super) async fn update_state<C>(
    state: &mut BroadcasterState,
    updated_entries: impl Iterator<Item = IndexedEntry>,
    io: &C,
) -> BroadcasterResult<()>
where
    C: BroadcasterIoContext,
{
    let unfinalized_entries: Vec<_> = updated_entries
        .filter(|entry| !entry.item().is_finalized() && entry.item().is_valid())
        .collect();

    let next_idx = io.get_next_tx_idx().await?;
    if next_idx < state.next_idx {
        return Err(BroadcasterError::InconsistentNextIdx {
            expected: state.next_idx,
            got: next_idx,
        });
    }

    let new_unfinalized_entries = fetch_unfinalized_entries(io, state.next_idx, next_idx).await?;

    state.unfinalized_entries = unfinalized_entries;
    state.unfinalized_entries.extend(new_unfinalized_entries);
    state.next_idx = next_idx;
    Ok(())
}

/// Returns unfinalized but valid [`L1TxEntry`]s from context-backed DB starting from index `from`
/// until `to` non-inclusive.
pub(super) async fn fetch_unfinalized_entries<C>(
    io: &C,
    from: u64,
    to: u64,
) -> BroadcasterResult<Vec<IndexedEntry>>
where
    C: BroadcasterIoContext,
{
    let mut unfinalized_entries = Vec::new();
    for idx in from..to {
        let Some(txentry) = io.get_tx_entry(idx).await? else {
            break;
        };

        if !txentry.is_valid() {
            // A superseded entry is a routine outcome of fee bumping, not a fault, and every one of
            // them is re-read on each restart.
            if matches!(txentry.status, L1TxStatus::Replaced { .. }) {
                debug!(%idx, status = ?txentry.status, "skipping superseded broadcaster entry");
            } else {
                error!(%idx, status = ?txentry.status, "invalid broadcaster entry in DB; skipping");
            }
            continue;
        }

        if txentry.is_finalized() {
            continue;
        }

        unfinalized_entries.push(IndexedEntry::new(idx, txentry));
    }
    Ok(unfinalized_entries)
}

#[cfg(test)]
mod test {
    use std::{
        collections::BTreeMap,
        future::Future,
        sync::{Arc, Mutex},
    };

    use bitcoin::{absolute::LockTime, Amount, FeeRate, Transaction, Txid};
    use proptest::prelude::*;
    use strata_db_types::l1_broadcast::{L1TxEntry, L1TxRbfInfo, L1TxStatus};
    use strata_identifiers::{test_utils::buf32_strategy, Buf32};
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::L1Height;
    use tokio::runtime::Builder;

    use super::*;
    use crate::{
        broadcaster::io::{
            BroadcasterIoContext, PublishTxOutcome, TxConfirmationInfo, TxLookupOutcome,
        },
        test_utils::gen_l1_tx_entry_with_status,
    };

    const TEST_REORG_DEPTH: u32 = 6;
    const TEST_GENESIS_L1_HEIGHT: L1Height = 0;

    #[derive(Clone, Debug)]
    enum MockTxLookupResult {
        Missing,
        Found(TxConfirmationInfo),
        RetryLater,
    }

    #[derive(Clone, Debug)]
    enum MockBroadcastResult {
        Published,
        AlreadyInMempool,
        InvalidInputs,
        RetryLater,
    }

    #[derive(Clone, Debug, Default)]
    struct MockIoContext {
        next_idx: u64,
        entries: BTreeMap<u64, L1TxEntry>,
        tx_lookup: BTreeMap<Txid, MockTxLookupResult>,
        broadcast_results: BTreeMap<Txid, MockBroadcastResult>,
        entries_by_id: BTreeMap<Buf32, L1TxEntry>,
        adoptions: Arc<Mutex<Vec<(Buf32, Buf32, L1TxStatus)>>>,
        adoption_applies: bool,
    }

    impl MockIoContext {
        fn with_tx_lookup(mut self, txid: Txid, result: MockTxLookupResult) -> Self {
            self.tx_lookup.insert(txid, result);
            self
        }

        fn with_broadcast_result(mut self, txid: Txid, result: MockBroadcastResult) -> Self {
            self.broadcast_results.insert(txid, result);
            self
        }

        fn with_entry_by_id(mut self, txid: Buf32, entry: L1TxEntry) -> Self {
            self.entries_by_id.insert(txid, entry);
            self
        }

        fn with_adoption_applying(mut self, applies: bool) -> Self {
            self.adoption_applies = applies;
            self
        }

        fn adoptions(&self) -> Vec<(Buf32, Buf32, L1TxStatus)> {
            self.adoptions.lock().unwrap().clone()
        }
    }

    impl BroadcasterIoContext for MockIoContext {
        async fn get_next_tx_idx(&self) -> BroadcasterResult<u64> {
            Ok(self.next_idx)
        }

        async fn get_tx_entry(&self, idx: u64) -> BroadcasterResult<Option<L1TxEntry>> {
            Ok(self.entries.get(&idx).cloned())
        }

        async fn put_tx_entry_by_idx(&self, _idx: u64, _entry: L1TxEntry) -> BroadcasterResult<()> {
            Ok(())
        }

        async fn get_tx_entry_by_id(&self, txid: Buf32) -> BroadcasterResult<Option<L1TxEntry>> {
            Ok(self.entries_by_id.get(&txid).cloned())
        }

        async fn adopt_confirmed_ancestor(
            &self,
            loser_txid: Buf32,
            winner_txid: Buf32,
            winner_status: L1TxStatus,
        ) -> BroadcasterResult<bool> {
            self.adoptions
                .lock()
                .unwrap()
                .push((loser_txid, winner_txid, winner_status));
            Ok(self.adoption_applies)
        }

        async fn get_transaction<'a>(
            &'a self,
            txid: &'a Txid,
        ) -> BroadcasterResult<TxLookupOutcome> {
            let result = self
                .tx_lookup
                .get(txid)
                .cloned()
                .unwrap_or(MockTxLookupResult::Missing);

            match result {
                MockTxLookupResult::Missing => Ok(TxLookupOutcome::Missing),
                MockTxLookupResult::Found(info) => Ok(TxLookupOutcome::Found(info)),
                MockTxLookupResult::RetryLater => Ok(TxLookupOutcome::RetryLater {
                    reason: "mock retry".into(),
                }),
            }
        }

        async fn send_raw_transaction<'a>(
            &'a self,
            tx: &'a Transaction,
        ) -> BroadcasterResult<PublishTxOutcome> {
            let txid = tx.compute_txid();
            let result = self
                .broadcast_results
                .get(&txid)
                .cloned()
                .unwrap_or(MockBroadcastResult::Published);

            match result {
                MockBroadcastResult::Published => Ok(PublishTxOutcome::Published),
                MockBroadcastResult::AlreadyInMempool => Ok(PublishTxOutcome::AlreadyInMempool),
                MockBroadcastResult::InvalidInputs => Ok(PublishTxOutcome::InvalidInputs),
                MockBroadcastResult::RetryLater => Ok(PublishTxOutcome::RetryLater {
                    reason: "mock retry".into(),
                }),
            }
        }
    }

    fn get_test_btcio_params() -> BtcioParams {
        BtcioParams::new(
            TEST_REORG_DEPTH,          // l1_reorg_safe_depth
            MagicBytes::new(*b"ALPN"), // magic_bytes
            TEST_GENESIS_L1_HEIGHT,    // genesis_l1_height
        )
    }

    fn entry_with_txid(status: L1TxStatus) -> (L1TxEntry, Txid) {
        let entry = gen_l1_tx_entry_with_status(status);
        let txid = entry.try_to_tx().unwrap().compute_txid();
        (entry, txid)
    }

    fn confirmation_info(
        confirmations: i64,
        block_height: L1Height,
        block_hash: Buf32,
    ) -> TxConfirmationInfo {
        if confirmations == 0 {
            TxConfirmationInfo {
                confirmations,
                block_hash: None,
                block_height: None,
            }
        } else {
            TxConfirmationInfo {
                confirmations,
                block_hash: Some(block_hash),
                block_height: Some(block_height),
            }
        }
    }

    fn status_with_confirmations(
        confirmations: u64,
        block_height: L1Height,
        block_hash: Buf32,
        finalized: bool,
    ) -> L1TxStatus {
        if finalized {
            L1TxStatus::Finalized {
                confirmations,
                block_hash,
                block_height,
            }
        } else {
            L1TxStatus::Confirmed {
                confirmations,
                block_hash,
                block_height,
            }
        }
    }

    fn confirmed_status(
        confirmations: u64,
        block_height: L1Height,
        block_hash: Buf32,
    ) -> L1TxStatus {
        status_with_confirmations(confirmations, block_height, block_hash, false)
    }

    fn finalized_status(
        confirmations: u64,
        block_height: L1Height,
        block_hash: Buf32,
    ) -> L1TxStatus {
        status_with_confirmations(confirmations, block_height, block_hash, true)
    }

    async fn process_status(
        io: &MockIoContext,
        entry: &L1TxEntry,
        txid: &Txid,
        params: &BtcioParams,
    ) -> Option<L1TxStatus> {
        process_tx_entry(io, entry, txid, params).await.unwrap()
    }

    fn run_async_test<F>(future: F)
    where
        F: Future<Output = ()>,
    {
        let runtime = Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(future);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io =
            MockIoContext::default().with_broadcast_result(txid, MockBroadcastResult::Published);

        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Status should be published for unpublished tx after successful broadcast"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry_status_500_keeps_unpublished() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io =
            MockIoContext::default().with_broadcast_result(txid, MockBroadcastResult::RetryLater);

        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Unpublished),
            "HTTP 500 send_raw_transaction errors should keep tx unpublished for retry"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry_server_minus22_marks_invalid_inputs() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_broadcast_result(txid, MockBroadcastResult::InvalidInputs);

        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::InvalidInputs),
            "Server(-22, ..) send_raw_transaction errors should mark tx invalid"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry_already_in_mempool_marks_published() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_broadcast_result(txid, MockBroadcastResult::AlreadyInMempool);

        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Server(-25, ..) send_raw_transaction should be treated as already published"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(12))]

        #[test]
        fn test_handle_published_entry(
            block_height in 1_u32..1_000_000,
            block_hash in buf32_strategy().prop_map(|buf| buf),
        ) {
            run_async_test(async move {
                let (e, txid) = entry_with_txid(L1TxStatus::Published);
                let btcio_params = get_test_btcio_params();
                let reorg_depth = i64::from(btcio_params.l1_reorg_safe_depth());

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(0, block_height, block_hash)),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(L1TxStatus::Published),
                    "Status should not change if no confirmations for a published tx"
                );

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(
                        reorg_depth - 1,
                        block_height,
                        block_hash,
                    )),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(confirmed_status(
                        (reorg_depth - 1) as u64,
                        block_height,
                        block_hash,
                    )),
                    "Status should be confirmed if 0 < confirmations < finality_depth"
                );

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(
                        reorg_depth,
                        block_height,
                        block_hash,
                    )),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(finalized_status(reorg_depth as u64, block_height, block_hash)),
                    "Status should be finalized if confirmations >= finality_depth"
                );
            });
        }

        #[test]
        fn test_handle_confirmed_entry(
            block_height in 1_u32..1_000_000,
            block_hash in buf32_strategy().prop_map(|buf| buf),
        ) {
            run_async_test(async move {
                let (e, txid) = entry_with_txid(confirmed_status(1, block_height, block_hash));
                let btcio_params = get_test_btcio_params();
                let reorg_depth = i64::from(btcio_params.l1_reorg_safe_depth());

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(0, block_height, block_hash)),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(L1TxStatus::Unpublished),
                    "Status should revert to unpublished if confirmed tx now has 0 confirmations"
                );

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(
                        reorg_depth - 1,
                        block_height,
                        block_hash,
                    )),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(confirmed_status(
                        (reorg_depth - 1) as u64,
                        block_height,
                        block_hash,
                    )),
                    "Status should remain confirmed if 0 < confirmations < finality_depth"
                );

                let io = MockIoContext::default().with_tx_lookup(
                    txid,
                    MockTxLookupResult::Found(confirmation_info(
                        reorg_depth,
                        block_height,
                        block_hash,
                    )),
                );
                let res = process_status(&io, &e, &txid, &btcio_params).await;
                assert_eq!(
                    res,
                    Some(finalized_status(reorg_depth as u64, block_height, block_hash)),
                    "Status should be finalized if confirmations >= finality_depth"
                );
            });
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_published_entry_missing_tx_holds_published() {
        // Bitcoind's `gettransaction` can briefly report a freshly broadcast
        // tx as missing before the wallet's chain syncer catches up. A
        // `Published` entry must not regress to `Unpublished` on that
        // transient miss; otherwise the broadcaster oscillates and the
        // watcher's curr_payloadidx never advances past it.
        let (e, txid) = entry_with_txid(L1TxStatus::Published);
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default().with_tx_lookup(txid, MockTxLookupResult::Missing);
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Published entry must hold its status when get_transaction returns NotFound"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_published_entry_dropped_from_mempool_advances_to_invalid_inputs() {
        // A `Published` entry whose lookup says "not found" AND whose
        // re-publish probe returns `InvalidInputs` (e.g. its inputs were
        // spent or evicted) must transition to `InvalidInputs` so the
        // watcher's `determine_payload_next_status` flips the bundle to
        // `NeedsResign` and the envelope is rebuilt against fresh UTXOs.
        // Without the publish-probe in `probe_published_entry`, the entry
        // would stay `Published` forever and the watcher's
        // `curr_payloadidx` would stall.
        let (e, txid) = entry_with_txid(L1TxStatus::Published);
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default()
            .with_tx_lookup(txid, MockTxLookupResult::Missing)
            .with_broadcast_result(txid, MockBroadcastResult::InvalidInputs);
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::InvalidInputs),
            "Published entry whose re-publish probe returns InvalidInputs must \
             transition to InvalidInputs so the watcher rebuilds the envelope"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_published_entry_already_in_mempool_holds_published() {
        // A `Published` entry whose lookup says "not found" but whose
        // re-publish probe returns `AlreadyInMempool` is in the transient
        // wallet-syncer-lag state. Stay `Published`; do not regress.
        let (e, txid) = entry_with_txid(L1TxStatus::Published);
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default()
            .with_tx_lookup(txid, MockTxLookupResult::Missing)
            .with_broadcast_result(txid, MockBroadcastResult::AlreadyInMempool);
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Published entry whose probe says already-in-mempool must hold"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_published_entry_at_zero_conf_does_not_republish() {
        // A `Published` entry that `gettransaction` reports as 0-conf in the
        // wallet is alive in mempool and waiting to be mined. Re-publishing
        // every poll for the entire pre-confirmation window would only spam
        // bitcoind and the broadcaster logs. The probe must hold at Published
        // without calling `send_raw_transaction`. We poison the broadcast path
        // so any re-publish would surface as `InvalidInputs`; the assertion
        // proves the probe never went down that road.
        let (e, txid) = entry_with_txid(L1TxStatus::Published);
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default()
            .with_tx_lookup(
                txid,
                MockTxLookupResult::Found(confirmation_info(0, 0, Buf32::zero())),
            )
            .with_broadcast_result(txid, MockBroadcastResult::InvalidInputs);
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Published entry found at 0-conf in wallet must hold without re-publishing"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_published_entry_lookup_retry_later_holds_published() {
        let (e, txid) = entry_with_txid(L1TxStatus::Published);
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default()
            .with_tx_lookup(txid, MockTxLookupResult::RetryLater)
            .with_broadcast_result(txid, MockBroadcastResult::InvalidInputs);
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "Published entry must hold on retryable get_transaction failure"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_confirmed_entry_missing_tx_regresses_to_unpublished() {
        // Confirmed entries that go missing on lookup should still regress to
        // Unpublished so the broadcaster re-publishes (e.g. after a reorg
        // dropped them from the wallet view).
        let (e, txid) = entry_with_txid(confirmed_status(1, 1, Buf32::zero()));
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default().with_tx_lookup(txid, MockTxLookupResult::Missing);
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Unpublished),
            "Confirmed entry that disappears should regress to Unpublished"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_confirmed_entry_lookup_retry_later_holds_confirmed() {
        let current_status = confirmed_status(1, 1, Buf32::zero());
        let (e, txid) = entry_with_txid(current_status.clone());
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default().with_tx_lookup(txid, MockTxLookupResult::RetryLater);
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(current_status),
            "Confirmed entry must hold on retryable get_transaction failure"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_finalized_entry() {
        let (e, txid) = entry_with_txid(finalized_status(1, 1, Buf32::zero()));
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default();
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(res, None, "Finalized tx should remain unchanged");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_excluded_entry() {
        let e = gen_l1_tx_entry_with_status(L1TxStatus::InvalidInputs);
        let btcio_params = get_test_btcio_params();
        let txid = e.try_to_tx().unwrap().compute_txid();

        let io = MockIoContext::default();
        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(res, None, "InvalidInputs tx should remain unchanged");
    }
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(12))]

        #[test]
        fn test_process_unfinalized_entries(
            seed_idx in 1_u64..1_000_000,
            block_height in 1_u32..1_000_000,
            block_hash in buf32_strategy().prop_map(|buf| buf),
        ) {
            run_async_test(async move {
                let btcio_params = get_test_btcio_params();
                let reorg_depth = btcio_params.l1_reorg_safe_depth() as u64;

                let (e1, txid1) = entry_with_txid(L1TxStatus::Unpublished);
                let i1 = seed_idx;

                let e2 = gen_l1_tx_entry_with_status(L1TxStatus::InvalidInputs);
                let i2 = seed_idx + 1;

                let (e3, txid3) = entry_with_txid(L1TxStatus::Published);
                let i3 = seed_idx + 2;

                let unfinalized_entries = [
                    IndexedEntry::new(i1, e1),
                    IndexedEntry::new(i2, e2),
                    IndexedEntry::new(i3, e3),
                ];

                let io = MockIoContext::default()
                    .with_broadcast_result(txid1, MockBroadcastResult::Published)
                    .with_tx_lookup(
                        txid3,
                        MockTxLookupResult::Found(confirmation_info(
                            reorg_depth as i64,
                            block_height,
                            block_hash,
                        )),
                    );

                let updated_entries = process_unfinalized_entries(
                    unfinalized_entries.iter(),
                    &io,
                    &btcio_params,
                )
                .await
                .unwrap();

                assert_eq!(
                    updated_entries
                        .iter()
                        .find(|e| *e.index() == i1)
                        .map(|e| e.item().status.clone())
                        .unwrap(),
                    L1TxStatus::Published,
                    "unpublished tx should be published"
                );
                assert_eq!(
                    updated_entries
                        .iter()
                        .find(|e| *e.index() == i3)
                        .map(|e| e.item().status.clone())
                        .unwrap(),
                    finalized_status(reorg_depth, block_height, block_hash),
                    "published tx should be finalized"
                );
            });
        }
    }

    // ── confirmation_status ──

    /// Builds a two-attempt chain: `winner` was superseded by `loser`, and the reverse link is set
    /// as `put_replacement_tx_entry` would have set it.
    fn replacement_chain() -> (L1TxEntry, Txid, L1TxEntry, Txid, Buf32) {
        let (mut winner, winner_txid) = entry_with_txid(L1TxStatus::Published);
        let winner_raw = Buf32(winner_txid.to_byte_array());
        winner.rbf = Some(L1TxRbfInfo {
            fee_rate_sat_vb: 2,
            fee_sats: 200,
            replaces: None,
        });

        // A distinct transaction, so the two entries do not collide by txid.
        let loser_tx = {
            let mut tx = winner.try_to_tx().unwrap();
            tx.lock_time = LockTime::from_consensus(7);
            tx
        };
        let loser_txid = loser_tx.compute_txid();
        let mut loser = L1TxEntry::from_tx_with_fee(
            &loser_tx,
            FeeRate::from_sat_per_vb(4).unwrap(),
            Amount::from_sat(400),
        );
        loser.status = L1TxStatus::Published;
        loser.set_replaces(L1TxId::from(winner_raw.0));

        winner.status = L1TxStatus::Replaced {
            by: L1TxId::from(loser_txid.to_byte_array()),
        };

        (winner, winner_txid, loser, loser_txid, winner_raw)
    }

    /// Regression: a miner can include the original after the local node accepted the replacement.
    /// Marking the replacement `InvalidInputs` sends the writer off to rebuild, republishing a
    /// payload the confirmed ancestor already carries.
    #[tokio::test(flavor = "multi_thread")]
    async fn negative_confirmations_adopt_a_confirmed_ancestor() {
        let (winner, winner_txid, loser, loser_txid, winner_raw) = replacement_chain();
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_entry_by_id(winner_raw, winner)
            .with_adoption_applying(true)
            .with_tx_lookup(
                loser_txid,
                MockTxLookupResult::Found(TxConfirmationInfo {
                    confirmations: -1,
                    block_hash: None,
                    block_height: None,
                }),
            )
            .with_tx_lookup(
                winner_txid,
                MockTxLookupResult::Found(confirmation_info(3, 400, Buf32::new([9u8; 32]))),
            );

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(
            status,
            Some(L1TxStatus::Replaced {
                by: L1TxId::from(winner_raw.0)
            }),
            "the loser must point at the winner, not be marked invalid"
        );
        let adoptions = io.adoptions();
        assert_eq!(adoptions.len(), 1);
        assert_eq!(adoptions[0].1, winner_raw);
        assert_eq!(
            adoptions[0].2,
            confirmed_status(3, 400, Buf32::new([9u8; 32]))
        );
    }

    /// A conflict with something outside the chain is a real double spend, and the original verdict
    /// stands.
    #[tokio::test(flavor = "multi_thread")]
    async fn negative_confirmations_stay_invalid_without_a_confirmed_ancestor() {
        let (winner, winner_txid, loser, loser_txid, winner_raw) = replacement_chain();
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_entry_by_id(winner_raw, winner)
            .with_adoption_applying(true)
            .with_tx_lookup(
                loser_txid,
                MockTxLookupResult::Found(TxConfirmationInfo {
                    confirmations: -1,
                    block_hash: None,
                    block_height: None,
                }),
            )
            // The ancestor did not win either.
            .with_tx_lookup(winner_txid, MockTxLookupResult::Missing);

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(status, Some(L1TxStatus::InvalidInputs));
        assert!(io.adoptions().is_empty());
    }

    /// Regression: every writer-owned entry carries RBF metadata from its first attempt, so the
    /// metadata alone must not be read as "in a replacement chain". A first attempt conflicting
    /// with the best chain is the ordinary side-branch sighting, and rebuilding it would republish
    /// a payload that gets mined twice once the conflicting transaction is reorged out.
    #[tokio::test(flavor = "multi_thread")]
    async fn negative_confirmations_hold_a_first_attempt_at_published() {
        let (mut entry, txid) = entry_with_txid(L1TxStatus::Published);
        entry.rbf = Some(L1TxRbfInfo {
            fee_rate_sat_vb: 2,
            fee_sats: 200,
            replaces: None,
        });
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default().with_tx_lookup(
            txid,
            MockTxLookupResult::Found(TxConfirmationInfo {
                confirmations: -1,
                block_hash: None,
                block_height: None,
            }),
        );

        let status = process_status(&io, &entry, &txid, &btcio_params).await;

        assert_eq!(status, Some(L1TxStatus::Published));
        assert!(io.adoptions().is_empty());
    }

    /// If the reversal did not apply the caller must not report the loser as invalid either: some
    /// other pass already resolved the chain.
    #[tokio::test(flavor = "multi_thread")]
    async fn negative_confirmations_stay_invalid_when_the_reversal_is_refused() {
        let (winner, winner_txid, loser, loser_txid, winner_raw) = replacement_chain();
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_entry_by_id(winner_raw, winner)
            .with_adoption_applying(false)
            .with_tx_lookup(
                loser_txid,
                MockTxLookupResult::Found(TxConfirmationInfo {
                    confirmations: -1,
                    block_hash: None,
                    block_height: None,
                }),
            )
            .with_tx_lookup(
                winner_txid,
                MockTxLookupResult::Found(confirmation_info(3, 400, Buf32::new([9u8; 32]))),
            );

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(status, Some(L1TxStatus::InvalidInputs));
        assert_eq!(io.adoptions().len(), 1);
    }

    /// Regression: the original can be mined between the replacement being persisted and its first
    /// broadcast. The replacement is then rejected for spent inputs on that very first send, so it
    /// never reaches `Published` and the negative-confirmation path never runs. Left as
    /// `InvalidInputs` the chain resolves to a dead entry and the writer rebuilds a payload the
    /// confirmed original already carries.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unpublished_replacement_rejected_for_spent_inputs_adopts_its_ancestor() {
        let (winner, winner_txid, mut loser, loser_txid, winner_raw) = replacement_chain();
        // Never broadcast: the writer persisted it and the block landed first.
        loser.status = L1TxStatus::Unpublished;
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_entry_by_id(winner_raw, winner)
            .with_adoption_applying(true)
            // The original took the inputs, so bitcoind rejects the replacement outright.
            .with_broadcast_result(loser_txid, MockBroadcastResult::InvalidInputs)
            .with_tx_lookup(
                winner_txid,
                MockTxLookupResult::Found(confirmation_info(1, 400, Buf32::new([9u8; 32]))),
            );

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(
            status,
            Some(L1TxStatus::Replaced {
                by: L1TxId::from(winner_raw.0)
            }),
            "a replacement rejected because its ancestor confirmed must adopt that ancestor"
        );
        let adoptions = io.adoptions();
        assert_eq!(adoptions.len(), 1);
        assert_eq!(adoptions[0].1, winner_raw);
    }

    /// The verdict still stands when the rejection is a real double spend from outside the chain.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_unpublished_replacement_stays_invalid_without_a_confirmed_ancestor() {
        let (winner, winner_txid, mut loser, loser_txid, winner_raw) = replacement_chain();
        loser.status = L1TxStatus::Unpublished;
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_entry_by_id(winner_raw, winner)
            .with_adoption_applying(true)
            .with_broadcast_result(loser_txid, MockBroadcastResult::InvalidInputs)
            .with_tx_lookup(winner_txid, MockTxLookupResult::Missing);

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(status, Some(L1TxStatus::InvalidInputs));
        assert!(io.adoptions().is_empty());
    }

    /// The re-publish probe reaches the same rejection by a different route: a `Published`
    /// replacement evicted from the wallet's view, re-sent, and refused because its ancestor won.
    /// The `Replaced` verdict must reach the caller instead of folding back to `Published`.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_republish_probe_rejected_for_spent_inputs_adopts_its_ancestor() {
        let (winner, winner_txid, loser, loser_txid, winner_raw) = replacement_chain();
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_entry_by_id(winner_raw, winner)
            .with_adoption_applying(true)
            .with_tx_lookup(loser_txid, MockTxLookupResult::Missing)
            .with_broadcast_result(loser_txid, MockBroadcastResult::InvalidInputs)
            .with_tx_lookup(
                winner_txid,
                MockTxLookupResult::Found(confirmation_info(1, 400, Buf32::new([9u8; 32]))),
            );

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(
            status,
            Some(L1TxStatus::Replaced {
                by: L1TxId::from(winner_raw.0)
            }),
        );
        assert_eq!(io.adoptions().len(), 1);
    }

    /// An entry outside a replacement chain has no ancestor to consult, so a rejection is still a
    /// rejection and the writer rebuilds as before.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_plain_entry_rejected_for_spent_inputs_stays_invalid() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        assert!(
            e.rbf.is_none(),
            "test: fixture must be outside an RBF chain"
        );
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_broadcast_result(txid, MockBroadcastResult::InvalidInputs);

        let status = process_status(&io, &e, &txid, &btcio_params).await;

        assert_eq!(status, Some(L1TxStatus::InvalidInputs));
        assert!(io.adoptions().is_empty());
    }

    #[test]
    fn confirmation_status_zero_confirmations_returns_published() {
        // 0-conf means the tx is in the mempool but not anchored to a block;
        // bitcoind leaves block_hash/block_height as None in that case.
        let info = TxConfirmationInfo {
            confirmations: 0,
            block_hash: None,
            block_height: None,
        };
        assert_eq!(confirmation_status(&info, 6), L1TxStatus::Published);
    }

    #[test]
    fn confirmation_status_negative_confirmations_returns_published() {
        // Negative confirmations mean the tx is on a side branch after a reorg;
        // treat it like 0-conf rather than synthesising a phantom Confirmed entry.
        let info = TxConfirmationInfo {
            confirmations: -2,
            block_hash: None,
            block_height: None,
        };
        assert_eq!(confirmation_status(&info, 6), L1TxStatus::Published);
    }

    #[test]
    fn confirmation_status_below_reorg_depth_is_confirmed() {
        let block_hash = Buf32::new([7u8; 32]);
        let block_height: L1Height = 100;
        let info = confirmation_info(3, block_height, block_hash);
        assert_eq!(
            confirmation_status(&info, 6),
            confirmed_status(3, block_height, block_hash),
        );
    }

    #[test]
    fn confirmation_status_at_or_above_reorg_depth_is_finalized() {
        let block_hash = Buf32::new([7u8; 32]);
        let block_height: L1Height = 100;
        let info = confirmation_info(6, block_height, block_hash);
        assert_eq!(
            confirmation_status(&info, 6),
            finalized_status(6, block_height, block_hash),
        );
    }
}
