use bitcoin::{hashes::Hash, Txid};
use strata_db_types::{
    common::L1TxId,
    l1_broadcast::{L1TxEntry, L1TxStatus},
};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::{
    error::{BroadcasterError, BroadcasterResult},
    handle::MAX_REPLACEMENT_CHAIN_HOPS,
    io::{
        BroadcasterIoContext, PublishDecision, PublishTxOutcome, TxConfirmationInfo,
        TxLookupOutcome,
    },
    state::{BroadcasterState, IndexedEntry},
};
use crate::{tx_entry::L1TxEntryExt, BtcioParams};

/// Result of one processing pass over the unfinalized set.
#[derive(Debug, Default)]
pub(super) struct ProcessingPassResult {
    /// Indexed entries whose status changed and must be written back.
    pub updated: Vec<IndexedEntry>,

    /// Whether the in-memory set has to be rebuilt from the full index range.
    ///
    /// Set when a superseded ancestor was adopted, either after winning on-chain or after the
    /// replacement that superseded it was refused while it was still in the mempool. That ancestor
    /// left `unfinalized_entries` when it was first replaced, and [`update_state`]'s cursor only
    /// reads rows added since, so without a rebuild nothing would ever poll it again: it would sit
    /// at its adopted status forever, never reaching `Finalized`, and a reorg on it would go
    /// unnoticed.
    pub resync_required: bool,
}

/// Processes unfinalized entries and returns the indexed entries whose status changed.
pub(super) async fn process_unfinalized_entries<C>(
    unfinalized_entries: impl Iterator<Item = &IndexedEntry>,
    io: &C,
    params: &BtcioParams,
) -> BroadcasterResult<ProcessingPassResult>
where
    C: BroadcasterIoContext,
{
    let mut processed = ProcessingPassResult::default();

    for entry in unfinalized_entries {
        let idx = *entry.index();
        let txentry = entry.item();
        let txid = txentry.try_to_tx()?.compute_txid();

        let updated_status = process_tx_entry(io, idx, txentry, &txid, params).await?;

        if let Some(status) = updated_status {
            // Adoption is the only path that turns a tracked entry into `Replaced`: the fee
            // bumper writes that status straight to the DB, and an already-`Replaced` entry
            // returns `None` above. So this is the signal that a winner needs re-admitting.
            if matches!(status, L1TxStatus::Replaced { .. }) {
                processed.resync_required = true;
            }
            let mut new_txentry = txentry.clone();
            new_txentry.status = status;
            io.put_tx_entry_by_idx(idx, new_txentry.clone()).await?;
            processed.updated.push(IndexedEntry::new(idx, new_txentry));
        }
    }

    Ok(processed)
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
    idx: u64,
    txentry: &L1TxEntry,
    txid: &Txid,
    params: &BtcioParams,
) -> BroadcasterResult<Option<L1TxStatus>>
where
    C: BroadcasterIoContext,
{
    let result = match txentry.status {
        L1TxStatus::Queued => publish_queued(io, idx, txentry, params).await.map(Some),
        L1TxStatus::Unpublished | L1TxStatus::Submitting => {
            probe_ambiguous_entry(io, idx, txentry, txid, params)
                .await
                .map(Some)
        }
        L1TxStatus::Published => probe_published_entry(io, txentry, txid, params)
            .await
            .map(Some),
        L1TxStatus::Confirmed { .. } => check_tx_confirmations(io, txentry, txid, params)
            .await
            .map(Some),
        L1TxStatus::Finalized { .. } => Ok(None),
        L1TxStatus::InvalidInputs | L1TxStatus::Abandoned | L1TxStatus::Replaced { .. } => Ok(None),
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
/// 3. `Ok(None)`: not found at all. Could be a transient wallet-syncer miss or a genuinely dropped
///    tx. Mark it as ambiguously submitted so recovery applies policy before any re-publication.
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
                return resolve_invalid_inputs(
                    io,
                    params,
                    txid,
                    txentry,
                    AncestorRescue::Confirmed,
                )
                .await;
            }
            Ok(confirmation_status(&info, reorg_safe_depth))
        }
        TxLookupOutcome::Missing => Ok(L1TxStatus::Submitting),
        TxLookupOutcome::RetryLater { reason } => {
            warn!(%reason, "transaction lookup should be retried on next poll");
            Ok(L1TxStatus::Published)
        }
    }
}

/// Which ancestors of a replacement chain count as a winner worth repointing the chain at.
#[derive(Clone, Copy, Debug)]
enum AncestorRescue {
    /// Only an ancestor already in the best chain.
    ///
    /// The verdict came from the entry conflicting with a confirmed transaction, and only a
    /// confirmed ancestor explains that sighting. An ancestor merely sitting in the mempool spends
    /// the same inputs, so it is just as doomed and repointing the chain at it would park the
    /// payload on a transaction that can never confirm.
    Confirmed,

    /// An ancestor already in the best chain, or failing that one still in the mempool.
    ///
    /// The verdict came from bitcoind refusing the broadcast itself, which proves nothing about
    /// the inputs: fee-policy rejects (`min relay fee not met`, `insufficient fee, rejecting
    /// replacement`, `txn-mempool-conflict`) and script-level rejects all arrive as
    /// [`PublishTxOutcome::InvalidInputs`]. While an ancestor is still live the chain has lost
    /// nothing, and the refused replacement must not become the verdict for the whole chain.
    ///
    /// [`PublishTxOutcome::InvalidInputs`]: super::io::PublishTxOutcome::InvalidInputs
    ConfirmedOrInMempool,
}

/// Settles an `InvalidInputs` verdict against the entry's own replacement chain.
///
/// `InvalidInputs` is what sends the writer off to rebuild, and rebuilding republishes a payload
/// another transaction in this very chain may already carry, or may still be about to carry. For an
/// entry in a replacement chain the transaction standing in its way is usually an ancestor: a miner
/// included an original after the local node had already accepted its replacement, or the node
/// refused the replacement and kept the original in its mempool.
///
/// Returns [`L1TxStatus::Replaced`] pointing at the ancestor the chain was repointed at. When no
/// ancestor qualifies under `rescue`, the conflict is with a transaction outside this chain and the
/// [`L1TxStatus::InvalidInputs`] verdict stands. An entry with no `replaces` link, which is every
/// entry outside a replacement chain, resolves to `InvalidInputs` without asking bitcoind anything.
async fn resolve_invalid_inputs<C>(
    io: &C,
    params: &BtcioParams,
    txid: &Txid,
    txentry: &L1TxEntry,
    rescue: AncestorRescue,
) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    if let Some(winner) = adopt_live_ancestor(io, params, txid, txentry, rescue).await? {
        return Ok(L1TxStatus::Replaced { by: winner });
    }
    Ok(L1TxStatus::InvalidInputs)
}

/// Repoints a replacement chain at a superseded ancestor that is still the chain's real tip.
///
/// A superseded ancestor is marked [`L1TxStatus::Replaced`], so the live-entry lookup walks
/// straight past it to the replacement and every consumer reads the replacement's verdict as the
/// chain's. Two situations make that reading wrong:
///
/// - The ancestor was mined anyway. Bitcoin Core reports negative confirmations for a transaction
///   conflicting with one already in the best chain; for a replacement chain that conflict is
///   usually its own ancestor, included by a miner after the local node had accepted the
///   replacement and evicted the original.
/// - The replacement was never accepted. A fee-policy or script-level reject leaves the ancestor
///   sitting in the mempool exactly as before, still able to confirm on its own.
///
/// Walks `replaces` back-pointers, asks bitcoind about each ancestor, and reverses the link so the
/// chain resolves to the winner. A confirmed ancestor is the stronger claim and can sit further
/// back than a mempool one, so the walk runs to completion before settling for the nearest ancestor
/// still in the mempool. Returns the winner's txid when the reversal applied, or `None` when no
/// ancestor qualifies and the caller's `InvalidInputs` verdict stands.
async fn adopt_live_ancestor<C>(
    io: &C,
    params: &BtcioParams,
    loser_txid: &Txid,
    loser_entry: &L1TxEntry,
    rescue: AncestorRescue,
) -> BroadcasterResult<Option<L1TxId>>
where
    C: BroadcasterIoContext,
{
    let reorg_safe_depth: i64 = params.l1_reorg_safe_depth().into();
    let mut ancestor = loser_entry.rbf.and_then(|rbf| rbf.replaces);
    let mut in_mempool: Option<(L1TxId, Txid)> = None;
    // Same budget as the forward walk in `L1BroadcastHandle`, so a chain the forward lookup
    // traverses is never one this search gives up on.
    let mut hops_left = MAX_REPLACEMENT_CHAIN_HOPS;

    while let Some(candidate) = ancestor {
        if hops_left == 0 {
            warn!(loser = %loser_txid, "conflicting-ancestor search exceeded its hop budget");
            break;
        }
        hops_left -= 1;

        let candidate_raw = Buf32(candidate.0);
        let Some(candidate_entry) = io.get_tx_entry_by_id(candidate_raw).await? else {
            break;
        };

        let candidate_txid = candidate_entry.try_to_tx()?.compute_txid();
        if let TxLookupOutcome::Found(info) = io.get_transaction(&candidate_txid).await? {
            if info.confirmations > 0 {
                let winner_status = confirmation_status(&info, reorg_safe_depth);
                info!(
                    loser = %loser_txid,
                    winner = %candidate_txid,
                    confirmations = info.confirmations,
                    "superseded transaction won on-chain; adopting it instead of rebuilding"
                );
                return apply_adoption(io, loser_txid, candidate, winner_status).await;
            }
            // Only a 0-conf sighting means the node still holds it in the mempool. Negative
            // confirmations mean this ancestor conflicts with the best chain too, so it is no
            // rescue for anyone.
            if info.confirmations == 0
                && matches!(rescue, AncestorRescue::ConfirmedOrInMempool)
                && in_mempool.is_none()
            {
                in_mempool = Some((candidate, candidate_txid));
            }
        }

        ancestor = candidate_entry.rbf.and_then(|rbf| rbf.replaces);
    }

    let Some((winner, winner_txid)) = in_mempool else {
        return Ok(None);
    };
    warn!(
        loser = %loser_txid,
        winner = %winner_txid,
        "replacement refused while the transaction it supersedes is still in the mempool; rolling the chain back to it"
    );
    apply_adoption(io, loser_txid, winner, L1TxStatus::Published).await
}

/// Reverses one replacement link, reporting the winner only when the write took effect.
async fn apply_adoption<C>(
    io: &C,
    loser_txid: &Txid,
    winner: L1TxId,
    winner_status: L1TxStatus,
) -> BroadcasterResult<Option<L1TxId>>
where
    C: BroadcasterIoContext,
{
    // `false` means another poll already reversed this pair, or the chain no longer links the two.
    // Stop either way rather than repointing at some older ancestor on a chain that has moved.
    if io
        .adopt_confirmed_ancestor(
            Buf32(loser_txid.to_byte_array()),
            Buf32(winner.0),
            winner_status,
        )
        .await?
    {
        return Ok(Some(winner));
    }
    Ok(None)
}

/// Recovers a transaction whose submission may have crossed a crash boundary.
async fn probe_ambiguous_entry<C>(
    io: &C,
    idx: u64,
    txentry: &L1TxEntry,
    txid: &Txid,
    params: &BtcioParams,
) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    let reorg_safe_depth: i64 = params.l1_reorg_safe_depth().into();
    match io.get_transaction(txid).await? {
        TxLookupOutcome::Found(info) => Ok(confirmation_status(&info, reorg_safe_depth)),
        TxLookupOutcome::Missing => {
            let tx = txentry.try_to_tx().expect("could not deserialize tx");
            match io.publish_decision(idx, &tx).await {
                PublishDecision::Publish => {
                    let mut submitting = txentry.clone();
                    submitting.status = L1TxStatus::Submitting;
                    io.put_tx_entry_by_idx(idx, submitting).await?;
                    submit_tx(io, params, txentry, L1TxStatus::Submitting).await
                }
                PublishDecision::Defer => Ok(txentry.status.clone()),
                PublishDecision::Abandon => Ok(L1TxStatus::Abandoned),
                PublishDecision::Invalidate => Ok(L1TxStatus::InvalidInputs),
            }
        }
        TxLookupOutcome::RetryLater { reason } => {
            warn!(%reason, "transaction lookup should be retried on next poll");
            Ok(txentry.status.clone())
        }
    }
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
/// `Published`, whose recovery path probes it again before consulting policy.
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
            TxLookupOutcome::Found(info) if info.confirmations == 0 => Ok(L1TxStatus::Published),
            TxLookupOutcome::Found(info) => Ok(confirmation_status(&info, reorg_safe_depth)),
            TxLookupOutcome::Missing => Ok(L1TxStatus::Published),
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

/// Applies policy and durably records that submission started before calling Bitcoin.
async fn publish_queued<C>(
    io: &C,
    idx: u64,
    txentry: &L1TxEntry,
    params: &BtcioParams,
) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    let tx = txentry.try_to_tx().expect("could not deserialize tx");
    match io.publish_decision(idx, &tx).await {
        PublishDecision::Publish => {}
        PublishDecision::Defer => return Ok(L1TxStatus::Queued),
        PublishDecision::Abandon => return Ok(L1TxStatus::Abandoned),
        PublishDecision::Invalidate => return Ok(L1TxStatus::InvalidInputs),
    }
    if tx.input.is_empty() {
        error!("tx has no inputs, excluding from broadcast");
        return Ok(L1TxStatus::InvalidInputs);
    }

    let mut submitting = txentry.clone();
    submitting.status = L1TxStatus::Submitting;
    io.put_tx_entry_by_idx(idx, submitting).await?;
    submit_tx(io, params, txentry, L1TxStatus::Submitting).await
}

/// Sends a transaction without consulting policy and settles RBF conflicts against its ancestors.
async fn submit_tx<C>(
    io: &C,
    params: &BtcioParams,
    txentry: &L1TxEntry,
    retry_status: L1TxStatus,
) -> BroadcasterResult<L1TxStatus>
where
    C: BroadcasterIoContext,
{
    let tx = txentry.try_to_tx().expect("could not deserialize tx");
    let txid = tx.compute_txid();
    let input_count = tx.input.len();
    let output_count = tx.output.len();

    async {
        debug!("publishing tx");
        match io.send_raw_transaction(&tx).await {
            Ok(PublishTxOutcome::Published | PublishTxOutcome::AlreadyInMempool) => {
                Ok(L1TxStatus::Published)
            }
            Ok(PublishTxOutcome::InvalidInputs) => {
                warn!("tx rejected on broadcast");
                resolve_invalid_inputs(
                    io,
                    params,
                    &txid,
                    txentry,
                    AncestorRescue::ConfirmedOrInMempool,
                )
                .await
            }
            Ok(PublishTxOutcome::AboveMaxFeeRate { reason }) => {
                warn!(%reason, "tx exceeds broadcast fee guardrail; rebuilding instead of retrying unchanged bytes");
                resolve_invalid_inputs(
                    io,
                    params,
                    &txid,
                    txentry,
                    AncestorRescue::ConfirmedOrInMempool,
                )
                .await
            }
            Ok(PublishTxOutcome::RetryLater { reason }) => {
                warn!(%reason, "broadcast should be retried on next poll");
                Ok(retry_status)
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
    resync_required: bool,
) -> BroadcasterResult<()>
where
    C: BroadcasterIoContext,
{
    let unfinalized_entries: Vec<_> = updated_entries
        .filter(|entry| !entry.item().is_finalized() && entry.item().is_trackable())
        .collect();

    let next_idx = io.get_next_tx_idx().await?;
    if next_idx < state.next_idx {
        return Err(BroadcasterError::InconsistentNextIdx {
            expected: state.next_idx,
            got: next_idx,
        });
    }

    if resync_required {
        // Rebuild from the whole range rather than the incremental window. The adopted winner
        // sits at an index below the cursor, so nothing else would pick it back up. Callers have
        // already written this pass's statuses back, so the DB is authoritative here.
        //
        // A full scan is affordable because adoption is a rare recovery path: it needs either a
        // superseded transaction to win on-chain, or a replacement to be refused outright, and the
        // replacement pass only builds one per transaction per bump interval.
        state.unfinalized_entries = fetch_unfinalized_entries(io, 0, next_idx).await?;
        state.next_idx = next_idx;
        return Ok(());
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

        if !txentry.is_trackable() {
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
        iter::once,
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
            BroadcasterIoContext, PublishDecision, PublishTxOutcome, TxConfirmationInfo,
            TxLookupOutcome,
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
        AboveMaxFeeRate,
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
        publish_decision: PublishDecision,
        persisted_statuses: Arc<Mutex<Vec<L1TxStatus>>>,
        require_submitting_before_send: bool,
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

        fn with_publish_decision(mut self, decision: PublishDecision) -> Self {
            self.publish_decision = decision;
            self
        }

        fn require_submitting_before_send(mut self) -> Self {
            self.require_submitting_before_send = true;
            self
        }
    }

    impl BroadcasterIoContext for MockIoContext {
        async fn publish_decision(&self, _: u64, _: &Transaction) -> PublishDecision {
            self.publish_decision
        }

        async fn get_next_tx_idx(&self) -> BroadcasterResult<u64> {
            Ok(self.next_idx)
        }

        async fn get_tx_entry(&self, idx: u64) -> BroadcasterResult<Option<L1TxEntry>> {
            Ok(self.entries.get(&idx).cloned())
        }

        async fn put_tx_entry_by_idx(&self, _idx: u64, entry: L1TxEntry) -> BroadcasterResult<()> {
            self.persisted_statuses.lock().unwrap().push(entry.status);
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
            if self.require_submitting_before_send {
                assert_eq!(
                    self.persisted_statuses.lock().unwrap().last(),
                    Some(&L1TxStatus::Submitting)
                );
            }
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
                MockBroadcastResult::AboveMaxFeeRate => Ok(PublishTxOutcome::AboveMaxFeeRate {
                    reason: "mock fee guardrail rejection".into(),
                }),
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

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_transaction_returns_typed_error() {
        let entry = IndexedEntry::new(
            0,
            L1TxEntry::from_raw_parts(vec![0xff], L1TxStatus::Published, None),
        );
        let io = MockIoContext::default();

        let err = process_unfinalized_entries(once(&entry), &io, &get_test_btcio_params())
            .await
            .unwrap_err();

        assert!(matches!(err, BroadcasterError::InvalidTransaction(_)));
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
        process_tx_entry(io, 0, entry, txid, params).await.unwrap()
    }

    async fn enter_missing_published_recovery(
        io: &MockIoContext,
        mut entry: L1TxEntry,
        txid: &Txid,
        params: &BtcioParams,
    ) -> L1TxEntry {
        entry.status = process_status(io, &entry, txid, params).await.unwrap();
        assert_eq!(entry.status, L1TxStatus::Submitting);
        entry
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
    async fn abandoned_pair_is_not_submitted_to_bitcoin() {
        let (commit, commit_txid) = entry_with_txid(L1TxStatus::Queued);
        let (reveal, reveal_txid) = entry_with_txid(L1TxStatus::Queued);
        let io = MockIoContext::default().with_publish_decision(PublishDecision::Abandon);
        let params = get_test_btcio_params();

        for (entry, txid) in [(commit, commit_txid), (reveal, reveal_txid)] {
            assert_eq!(
                process_status(&io, &entry, &txid, &params).await,
                Some(L1TxStatus::Abandoned)
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry_status_500_keeps_submitting() {
        let (e, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_broadcast_result(txid, MockBroadcastResult::RetryLater)
            .require_submitting_before_send();

        let res = process_status(&io, &e, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Submitting),
            "retryable send errors must preserve that submission started"
        );
        assert_eq!(
            *io.persisted_statuses.lock().unwrap(),
            [L1TxStatus::Submitting]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_handle_unpublished_entry_above_max_fee_requires_rebuild() {
        let (entry, txid) = entry_with_txid(L1TxStatus::Unpublished);
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_broadcast_result(txid, MockBroadcastResult::AboveMaxFeeRate);

        let status = process_status(&io, &entry, &txid, &btcio_params).await;

        assert_eq!(status, Some(L1TxStatus::InvalidInputs));
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
                    Some(L1TxStatus::Published),
                    "A reorged confirmed tx should return to publication recovery"
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
    async fn missing_published_entry_uses_recovery_policy() {
        let (e, txid) = entry_with_txid(L1TxStatus::Published);
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default()
            .with_tx_lookup(txid, MockTxLookupResult::Missing)
            .with_publish_decision(PublishDecision::Abandon);
        let e = enter_missing_published_recovery(&io, e, &txid, &btcio_params).await;
        assert_eq!(
            process_status(&io, &e, &txid, &btcio_params).await,
            Some(L1TxStatus::Abandoned)
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
        let e = enter_missing_published_recovery(&io, e, &txid, &btcio_params).await;
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
        let e = enter_missing_published_recovery(&io, e, &txid, &btcio_params).await;
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
    async fn missing_confirmed_entry_is_republished_after_reorg() {
        let (mut entry, txid) = entry_with_txid(confirmed_status(1, 1, Buf32::zero()));
        let btcio_params = get_test_btcio_params();

        let io = MockIoContext::default()
            .with_tx_lookup(txid, MockTxLookupResult::Missing)
            .with_broadcast_result(txid, MockBroadcastResult::Published);
        let res = process_status(&io, &entry, &txid, &btcio_params).await;
        assert_eq!(
            res,
            Some(L1TxStatus::Published),
            "A missing confirmed tx should return to publication recovery"
        );
        entry.status = res.unwrap();
        entry = enter_missing_published_recovery(&io, entry, &txid, &btcio_params).await;
        assert_eq!(
            process_status(&io, &entry, &txid, &btcio_params).await,
            Some(L1TxStatus::Published),
            "publication recovery must re-submit after applying policy"
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

                let (e1, txid1) = entry_with_txid(L1TxStatus::Queued);
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
                        .updated
                        .iter()
                        .find(|e| *e.index() == i1)
                        .map(|e| e.item().status.clone())
                        .unwrap(),
                    L1TxStatus::Published,
                    "unpublished tx should be published"
                );
                assert_eq!(
                    updated_entries
                        .updated
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

    /// A two-attempt replacement chain, named for how these tests end up using it: `winner` is the
    /// older attempt that a miner may still include, `loser` is the replacement that superseded it.
    struct ReplacementChain {
        winner: L1TxEntry,
        winner_txid: Txid,
        loser: L1TxEntry,
        loser_txid: Txid,
        /// `winner_txid` as the raw key the broadcast DB is indexed by.
        winner_raw: Buf32,
    }

    /// Builds a two-attempt chain: `winner` was superseded by `loser`, and the reverse link is set
    /// as `put_replacement_tx_entry` would have set it.
    fn replacement_chain() -> ReplacementChain {
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

        ReplacementChain {
            winner,
            winner_txid,
            loser,
            loser_txid,
            winner_raw,
        }
    }

    /// Regression: a miner can include the original after the local node accepted the replacement.
    /// Marking the replacement `InvalidInputs` sends the writer off to rebuild, republishing a
    /// payload the confirmed ancestor already carries.
    #[tokio::test(flavor = "multi_thread")]
    async fn negative_confirmations_adopt_a_confirmed_ancestor() {
        let ReplacementChain {
            winner,
            winner_txid,
            loser,
            loser_txid,
            winner_raw,
        } = replacement_chain();
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
        let ReplacementChain {
            winner,
            winner_txid,
            loser,
            loser_txid,
            winner_raw,
        } = replacement_chain();
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

    /// A refused reversal leaves the chain exactly as it was, so the `InvalidInputs` verdict
    /// stands and the caller still reports it. When the refusal was caused by a concurrent pass
    /// resolving the chain, that verdict never reaches the DB either: the `replaced_concurrently`
    /// guard in `state.rs` drops the write-back once it re-reads the row as `Replaced`.
    #[tokio::test(flavor = "multi_thread")]
    async fn negative_confirmations_stay_invalid_when_the_reversal_is_refused() {
        let ReplacementChain {
            winner,
            winner_txid,
            loser,
            loser_txid,
            winner_raw,
        } = replacement_chain();
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
        let ReplacementChain {
            winner,
            winner_txid,
            mut loser,
            loser_txid,
            winner_raw,
        } = replacement_chain();
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
        let ReplacementChain {
            winner,
            winner_txid,
            mut loser,
            loser_txid,
            winner_raw,
        } = replacement_chain();
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
        let ReplacementChain {
            winner,
            winner_txid,
            loser,
            loser_txid,
            winner_raw,
        } = replacement_chain();
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

        let loser = enter_missing_published_recovery(&io, loser, &loser_txid, &btcio_params).await;
        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(
            status,
            Some(L1TxStatus::Replaced {
                by: L1TxId::from(winner_raw.0)
            }),
        );
        assert_eq!(io.adoptions().len(), 1);
    }

    /// Builds a three-attempt chain, oldest first, wired the way `put_replacement_tx_entry` wires
    /// one: each entry is `Replaced` by the next and each replacement carries the reverse link.
    fn three_attempt_chain() -> Vec<(L1TxEntry, Txid)> {
        let (base, _) = entry_with_txid(L1TxStatus::Published);
        let base_tx = base.try_to_tx().unwrap();

        let mut attempts: Vec<(L1TxEntry, Txid)> = (0..3)
            .map(|i| {
                // Distinct lock times, so the three entries do not collide by txid.
                let mut tx = base_tx.clone();
                tx.lock_time = LockTime::from_consensus(11 + i);
                let txid = tx.compute_txid();
                let mut entry = L1TxEntry::from_tx_with_fee(
                    &tx,
                    FeeRate::from_sat_per_vb(2 + u64::from(i)).unwrap(),
                    Amount::from_sat(200 * (u64::from(i) + 1)),
                );
                entry.status = L1TxStatus::Published;
                (entry, txid)
            })
            .collect();

        for i in 0..attempts.len() - 1 {
            let (older_txid, newer_txid) = (attempts[i].1, attempts[i + 1].1);
            attempts[i].0.status = L1TxStatus::Replaced {
                by: L1TxId::from(newer_txid.to_byte_array()),
            };
            attempts[i + 1]
                .0
                .set_replaces(L1TxId::from(older_txid.to_byte_array()));
        }

        attempts
    }

    /// Regression: bitcoind rejects a replacement on fee policy (`min relay fee not met`,
    /// `insufficient fee, rejecting replacement`) or on script grounds, and the IO layer reports
    /// every one of those as `InvalidInputs`. The inputs are untouched and the original is still in
    /// the mempool, but the original row is already `Replaced`, so leaving the refused replacement
    /// as the chain's verdict makes the writer rebuild and repost the DA a live transaction still
    /// carries. Roll the chain back to the original instead, where the fee bumper can escalate it
    /// again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_replacement_refused_on_broadcast_rolls_back_to_its_live_original() {
        let ReplacementChain {
            winner,
            winner_txid,
            mut loser,
            loser_txid,
            winner_raw,
        } = replacement_chain();
        loser.status = L1TxStatus::Unpublished;
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_entry_by_id(winner_raw, winner)
            .with_adoption_applying(true)
            .with_broadcast_result(loser_txid, MockBroadcastResult::InvalidInputs)
            // The original never left the mempool: bitcoind simply refused to swap it out.
            .with_tx_lookup(
                winner_txid,
                MockTxLookupResult::Found(confirmation_info(0, 0, Buf32::zero())),
            );

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(
            status,
            Some(L1TxStatus::Replaced {
                by: L1TxId::from(winner_raw.0)
            }),
            "a refused replacement must not outlive the original it never replaced"
        );
        let adoptions = io.adoptions();
        assert_eq!(adoptions.len(), 1);
        assert_eq!(adoptions[0].1, winner_raw);
        assert_eq!(
            adoptions[0].2,
            L1TxStatus::Published,
            "the original goes back to the status it held in the mempool"
        );
    }

    /// An ancestor at negative confirmations conflicts with the best chain just as the loser does,
    /// so it is no rescue: the chain really is dead and the writer must rebuild.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_replacement_refused_stays_invalid_when_its_ancestor_conflicts_too() {
        let ReplacementChain {
            winner,
            winner_txid,
            mut loser,
            loser_txid,
            winner_raw,
        } = replacement_chain();
        loser.status = L1TxStatus::Unpublished;
        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_entry_by_id(winner_raw, winner)
            .with_adoption_applying(true)
            .with_broadcast_result(loser_txid, MockBroadcastResult::InvalidInputs)
            .with_tx_lookup(
                winner_txid,
                MockTxLookupResult::Found(TxConfirmationInfo {
                    confirmations: -1,
                    block_hash: None,
                    block_height: None,
                }),
            );

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(status, Some(L1TxStatus::InvalidInputs));
        assert!(io.adoptions().is_empty());
    }

    /// A tx conflicting with the best chain can only have lost to a confirmed transaction, so a
    /// mempool ancestor must not rescue it. That ancestor spends the same inputs and is therefore
    /// just as doomed; adopting it would park the payload on a transaction that can never confirm.
    #[tokio::test(flavor = "multi_thread")]
    async fn negative_confirmations_do_not_adopt_a_mempool_ancestor() {
        let ReplacementChain {
            winner,
            winner_txid,
            loser,
            loser_txid,
            winner_raw,
        } = replacement_chain();
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
                MockTxLookupResult::Found(confirmation_info(0, 0, Buf32::zero())),
            );

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(status, Some(L1TxStatus::InvalidInputs));
        assert!(io.adoptions().is_empty());
    }

    /// A confirmed ancestor is the stronger claim and can sit further back than a mempool one, so
    /// the walk must reach it rather than settling for the nearest live ancestor. Adopting the
    /// mempool one would leave the chain waiting on a transaction the confirmed ancestor has
    /// already made unspendable.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_confirmed_ancestor_outranks_a_nearer_mempool_one() {
        let chain = three_attempt_chain();
        let [(confirmed, confirmed_txid), (mempool, mempool_txid), (mut loser, loser_txid)] =
            <[_; 3]>::try_from(chain).unwrap();
        let confirmed_raw = Buf32(confirmed_txid.to_byte_array());
        loser.status = L1TxStatus::Unpublished;

        let btcio_params = get_test_btcio_params();
        let io = MockIoContext::default()
            .with_entry_by_id(confirmed_raw, confirmed)
            .with_entry_by_id(Buf32(mempool_txid.to_byte_array()), mempool)
            .with_adoption_applying(true)
            .with_broadcast_result(loser_txid, MockBroadcastResult::InvalidInputs)
            .with_tx_lookup(
                mempool_txid,
                MockTxLookupResult::Found(confirmation_info(0, 0, Buf32::zero())),
            )
            .with_tx_lookup(
                confirmed_txid,
                MockTxLookupResult::Found(confirmation_info(2, 400, Buf32::new([9u8; 32]))),
            );

        let status = process_status(&io, &loser, &loser_txid, &btcio_params).await;

        assert_eq!(
            status,
            Some(L1TxStatus::Replaced {
                by: L1TxId::from(confirmed_raw.0)
            }),
        );
        let adoptions = io.adoptions();
        assert_eq!(adoptions.len(), 1);
        assert_eq!(adoptions[0].1, confirmed_raw);
        assert_eq!(
            adoptions[0].2,
            confirmed_status(2, 400, Buf32::new([9u8; 32]))
        );
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
