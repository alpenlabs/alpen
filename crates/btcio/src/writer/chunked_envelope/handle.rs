//! Chunked envelope handle and lifecycle driver.
//!
//! Polls entries by sequential index and advances them through the status
//! lifecycle: Unsigned → Unpublished → CommitPublished → Published → Confirmed → Finalized.
//!
//! The `CommitPublished` intermediate state ensures reveal txs are broadcast directly
//! (all at once) when the commit tx is on-chain. For single-reveal entries, broadcast
//! happens as soon as the commit is in the mempool. For multi-reveal entries, we wait
//! for the commit to be confirmed in a block to avoid hitting Bitcoin Core's mempool
//! descendant size limit (default 101 KB).

use std::{collections::BTreeSet, future::Future, sync::Arc, time::Duration};

use bitcoin::{consensus::encode::deserialize as btc_deserialize, Address, Transaction};
use bitcoind_async_client::{
    error::ClientError,
    traits::{Broadcaster, Reader, Signer, Wallet},
    Client,
};
use strata_config::btcio::WriterConfig;
use strata_db_types::types::{ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, L1TxEntry, L1TxStatus};
use strata_primitives::buf::Buf32;
use strata_storage::ops::chunked_envelope::ChunkedEnvelopeOps;
use thiserror::Error;
use tokio::time::interval;
use tracing::*;

use super::{context::ChunkedWriterContext, signer::sign_chunked_envelope};
use crate::{broadcaster::L1BroadcastHandle, writer::builder::EnvelopeError, BtcioParams};

/// Maximum number of envelope rows to fetch per storage scan batch.
///
/// Recovery and tip-ingestion walk the DB in ordered chunks so they can
/// validate that indices are contiguous without materializing an arbitrarily
/// large range in a single call.
const ENTRY_SCAN_BATCH_SIZE: usize = 1_024;

/// Errors raised by chunked-envelope watcher recovery and polling.
///
/// These represent persisted-state invariants that should never be violated
/// during normal operation.
#[derive(Debug, Error)]
enum ChunkedEnvelopeWatcherError {
    /// The observed next row index moved backward relative to the watcher's tip.
    #[error(
        "chunked envelope next index regressed from {expected_next_idx} to {observed_next_idx}"
    )]
    NextIndexRegressed {
        expected_next_idx: u64,
        observed_next_idx: u64,
    },
    /// A contiguous scan skipped a persisted row before the known tip.
    #[error("chunked envelope entry gap at index {missing_idx}")]
    EntryGap { missing_idx: u64 },
}

/// Handle for submitting chunked envelope entries.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have debug impls"
)]
pub struct ChunkedEnvelopeHandle {
    ops: Arc<ChunkedEnvelopeOps>,
}

impl ChunkedEnvelopeHandle {
    pub fn new(ops: Arc<ChunkedEnvelopeOps>) -> Self {
        Self { ops }
    }

    /// Stores a new unsigned entry and returns the assigned index.
    pub async fn submit_entry(&self, entry: ChunkedEnvelopeEntry) -> anyhow::Result<u64> {
        let idx = self.ops.get_next_chunked_envelope_idx_async().await?;
        self.ops
            .put_chunked_envelope_entry_async(idx, entry)
            .await?;
        debug!(%idx, "submitted chunked envelope entry");
        Ok(idx)
    }

    /// Blocking variant of [`submit_entry`](Self::submit_entry).
    pub fn submit_entry_blocking(&self, entry: ChunkedEnvelopeEntry) -> anyhow::Result<u64> {
        let idx = self.ops.get_next_chunked_envelope_idx_blocking()?;
        self.ops.put_chunked_envelope_entry_blocking(idx, entry)?;
        debug!(%idx, "submitted chunked envelope entry");
        Ok(idx)
    }

    /// Returns the inner ops for direct reads.
    pub fn ops(&self) -> &Arc<ChunkedEnvelopeOps> {
        &self.ops
    }
}

/// Creates the chunked envelope lifecycle driver.
///
/// Returns a `(handle, future)` pair. The caller is responsible for spawning the
/// future on whatever executor it uses (e.g. alpen ee `task_executor`).
pub fn create_chunked_envelope_task(
    bitcoin_client: Arc<Client>,
    config: Arc<WriterConfig>,
    btcio_params: BtcioParams,
    sequencer_address: Address,
    ops: Arc<ChunkedEnvelopeOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> anyhow::Result<(Arc<ChunkedEnvelopeHandle>, impl Future<Output = ()>)> {
    let watcher_state = ChunkedEnvelopeWatcherState::recover(ops.as_ref())?;
    let handle = Arc::new(ChunkedEnvelopeHandle::new(ops.clone()));

    let ctx = Arc::new(ChunkedWriterContext::new(
        btcio_params,
        config,
        sequencer_address,
        bitcoin_client,
    ));

    let task = async move {
        if let Err(e) = watcher_task(watcher_state, ctx, ops, broadcast_handle).await {
            error!(%e, "chunked envelope watcher exited with error");
        }
    };

    Ok((handle, task))
}

/// In-memory scheduler state for the chunked envelope watcher.
///
/// The watcher tracks all non-finalized envelopes independently so older
/// entries can continue toward finality while a separate frontier decides when
/// the next unsigned entry may be signed.
#[derive(Debug)]
struct ChunkedEnvelopeWatcherState {
    /// Next DB index expected if a newly submitted entry appears.
    next_db_idx: u64,

    /// Earliest index whose successor dependency may still block new signing.
    forward_frontier: u64,

    /// Non-finalized envelope indices that still need status reconciliation.
    active_envelopes: BTreeSet<u64>,
}

impl ChunkedEnvelopeWatcherState {
    /// Rebuilds watcher state from the persisted chunked-envelope rows.
    ///
    /// Startup recovery scans the DB from index 0 up to the current tip and
    /// rejects gaps so the watcher does not silently skip corrupted entries.
    fn recover(ops: &ChunkedEnvelopeOps) -> anyhow::Result<Self> {
        let next_db_idx = ops.get_next_chunked_envelope_idx_blocking()?;
        let entries = load_entries_range_blocking(ops, 0, next_db_idx)?;
        Ok(Self::from_entries(next_db_idx, &entries))
    }

    /// Derives active entries and the signing frontier from a recovered row set.
    fn from_entries(next_db_idx: u64, entries: &[(u64, ChunkedEnvelopeEntry)]) -> Self {
        let active_envelopes = entries
            .iter()
            .filter(|(_, entry)| entry.status != ChunkedEnvelopeStatus::Finalized)
            .map(|(idx, _)| *idx)
            .collect();
        let forward_frontier = entries
            .iter()
            .find(|(_, entry)| !entry_unlocks_successor(entry))
            .map(|(idx, _)| *idx)
            .unwrap_or(next_db_idx);

        Self {
            next_db_idx,
            forward_frontier,
            active_envelopes,
        }
    }

    /// Incorporates newly appended DB rows into the active in-memory watcher state.
    ///
    /// This preserves the current tip index, rejects regressions, and enrolls
    /// any new non-finalized envelopes for reconciliation on the next tick.
    async fn ingest_new_entries(&mut self, ops: &ChunkedEnvelopeOps) -> anyhow::Result<()> {
        let observed_next_idx = ops.get_next_chunked_envelope_idx_async().await?;
        if observed_next_idx < self.next_db_idx {
            return Err(ChunkedEnvelopeWatcherError::NextIndexRegressed {
                expected_next_idx: self.next_db_idx,
                observed_next_idx,
            }
            .into());
        }

        if observed_next_idx == self.next_db_idx {
            return Ok(());
        }

        let entries = load_entries_range_async(ops, self.next_db_idx, observed_next_idx).await?;
        for (idx, entry) in entries {
            if entry.status != ChunkedEnvelopeStatus::Finalized {
                self.active_envelopes.insert(idx);
            }
        }
        self.next_db_idx = observed_next_idx;
        Ok(())
    }
}

fn format_reveal_refs(entry: &ChunkedEnvelopeEntry) -> Vec<String> {
    entry
        .reveals
        .iter()
        .map(|reveal| format!("{}/{}", reveal.txid, reveal.wtxid))
        .collect()
}

fn format_tx_status(txid: Buf32, status: &L1TxStatus) -> String {
    match status {
        L1TxStatus::Unpublished => format!("{txid}:unpublished"),
        L1TxStatus::Published => format!("{txid}:published"),
        L1TxStatus::InvalidInputs => format!("{txid}:invalid_inputs"),
        L1TxStatus::Confirmed {
            confirmations,
            block_hash,
            block_height,
        } => {
            format!("{txid}:confirmed@{block_height}/{block_hash} ({confirmations} confs)")
        }
        L1TxStatus::Finalized {
            confirmations,
            block_hash,
            block_height,
        } => {
            format!("{txid}:finalized@{block_height}/{block_hash} ({confirmations} confs)")
        }
    }
}

/// Polls entries and drives them through signing, broadcast, and confirmation.
///
/// The lifecycle is:
/// 1. `Unsigned`/`NeedsResign` → sign commit+reveals, store commit in broadcast DB → `Unpublished`
/// 2. `Unpublished` → wait for commit to be on-chain, then broadcast ALL reveals →
///    `CommitPublished`
/// 3. `CommitPublished` → wait for all reveals to be published → `Published`
/// 4. `Published` → wait for confirmation → `Confirmed`
/// 5. `Confirmed` → wait for finalization → `Finalized`
async fn watcher_task<R: Reader + Signer + Wallet + Broadcaster>(
    mut state: ChunkedEnvelopeWatcherState,
    ctx: Arc<ChunkedWriterContext<R>>,
    ops: Arc<ChunkedEnvelopeOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> anyhow::Result<()> {
    info!("starting chunked envelope watcher");
    let tick = interval(Duration::from_millis(ctx.config.write_poll_dur_ms));
    tokio::pin!(tick);
    let mut iteration = 0_u64;

    loop {
        tick.as_mut().tick().await;
        let next_db_idx = state.next_db_idx;
        let forward_frontier = state.forward_frontier;
        let active_envelopes = state.active_envelopes.len();
        async {
            state.ingest_new_entries(ops.as_ref()).await?;
            reconcile_active_entries(
                &mut state,
                ctx.client.as_ref(),
                ops.as_ref(),
                &broadcast_handle,
            )
            .await?;
            advance_forward_frontier(&mut state, ctx.clone(), ops.as_ref(), &broadcast_handle).await
        }
        .instrument(info_span!(
            "chunked_envelope_watcher_iteration",
            iteration,
            next_db_idx,
            forward_frontier,
            active_envelopes,
        ))
        .await?;
        iteration += 1;
    }
}

/// Returns `true` once an entry has enough persisted transaction data for its successor.
///
/// A successor only needs the predecessor's final reveal wtxid, so the entry
/// becomes a valid predecessor as soon as it has been signed and is no longer
/// in an unsigned/re-sign state.
fn entry_unlocks_successor(entry: &ChunkedEnvelopeEntry) -> bool {
    !matches!(
        entry.status,
        ChunkedEnvelopeStatus::Unsigned | ChunkedEnvelopeStatus::NeedsResign
    ) && !entry.reveals.is_empty()
}

/// Builds the canonical corruption error for a missing or skipped envelope row.
fn invalid_gap_error(missing_idx: u64) -> ChunkedEnvelopeWatcherError {
    ChunkedEnvelopeWatcherError::EntryGap { missing_idx }
}

/// Loads a contiguous envelope row range during startup recovery.
///
/// Any missing index below the observed tip is treated as corruption instead of
/// "nothing to do", because later entries may still exist and would otherwise
/// be skipped forever.
fn load_entries_range_blocking(
    ops: &ChunkedEnvelopeOps,
    start_idx: u64,
    end_idx: u64,
) -> anyhow::Result<Vec<(u64, ChunkedEnvelopeEntry)>> {
    let mut entries = Vec::new();
    let mut cursor = start_idx;
    while cursor < end_idx {
        let remaining = usize::try_from(end_idx - cursor).unwrap_or(usize::MAX);
        let batch = ops.get_chunked_envelope_entries_from_blocking(
            cursor,
            remaining.min(ENTRY_SCAN_BATCH_SIZE),
        )?;
        if batch.is_empty() {
            return Err(invalid_gap_error(cursor).into());
        }

        for (idx, entry) in batch {
            if idx != cursor {
                return Err(invalid_gap_error(cursor).into());
            }
            entries.push((idx, entry));
            cursor += 1;
        }
    }

    Ok(entries)
}

/// Async variant of [`load_entries_range_blocking`] used while the watcher is running.
///
/// This is used when ingesting new rows that appeared since the last poll tick.
async fn load_entries_range_async(
    ops: &ChunkedEnvelopeOps,
    start_idx: u64,
    end_idx: u64,
) -> anyhow::Result<Vec<(u64, ChunkedEnvelopeEntry)>> {
    let mut entries = Vec::new();
    let mut cursor = start_idx;
    while cursor < end_idx {
        let remaining = usize::try_from(end_idx - cursor).unwrap_or(usize::MAX);
        let batch = ops
            .get_chunked_envelope_entries_from_async(cursor, remaining.min(ENTRY_SCAN_BATCH_SIZE))
            .await?;
        if batch.is_empty() {
            return Err(invalid_gap_error(cursor).into());
        }

        for (idx, entry) in batch {
            if idx != cursor {
                return Err(invalid_gap_error(cursor).into());
            }
            entries.push((idx, entry));
            cursor += 1;
        }
    }

    Ok(entries)
}

/// Reconciles every active non-finalized envelope against broadcast-layer state.
///
/// This lets older envelopes continue progressing toward finality even when the
/// signing frontier has moved on to later queue items. Entries that regress
/// back to `Unsigned` or `NeedsResign` pull the frontier back so successors are
/// not allowed to outpace their predecessor dependency.
async fn reconcile_active_entries(
    state: &mut ChunkedEnvelopeWatcherState,
    client: &impl Broadcaster,
    ops: &ChunkedEnvelopeOps,
    broadcast_handle: &L1BroadcastHandle,
) -> anyhow::Result<()> {
    let active_indices: Vec<u64> = state.active_envelopes.iter().copied().collect();
    for idx in active_indices {
        let Some(entry) = ops.get_chunked_envelope_entry_async(idx).await? else {
            return Err(invalid_gap_error(idx).into());
        };

        let new_status = match entry.status {
            ChunkedEnvelopeStatus::Finalized => {
                state.active_envelopes.remove(&idx);
                continue;
            }
            ChunkedEnvelopeStatus::Unsigned | ChunkedEnvelopeStatus::NeedsResign => {
                state.forward_frontier = state.forward_frontier.min(idx);
                continue;
            }
            ChunkedEnvelopeStatus::Unpublished => {
                check_commit_and_broadcast_reveals(idx, &entry, broadcast_handle, client).await?
            }
            ChunkedEnvelopeStatus::CommitPublished
            | ChunkedEnvelopeStatus::Published
            | ChunkedEnvelopeStatus::Confirmed => {
                check_full_broadcast_status(idx, &entry, broadcast_handle).await?
            }
        };

        if new_status != entry.status {
            let reveal_refs = format_reveal_refs(&entry);
            debug!(
                envelope_idx = idx,
                commit_txid = %entry.commit_txid,
                ?reveal_refs,
                old_status = ?entry.status,
                ?new_status,
                "entry status changed"
            );
            let mut updated = entry;
            updated.status = new_status.clone();
            ops.put_chunked_envelope_entry_async(idx, updated).await?;
            if matches!(
                new_status,
                ChunkedEnvelopeStatus::Unsigned | ChunkedEnvelopeStatus::NeedsResign
            ) {
                state.forward_frontier = state.forward_frontier.min(idx);
            }
        }

        if new_status == ChunkedEnvelopeStatus::Finalized {
            state.active_envelopes.remove(&idx);
        }
    }

    Ok(())
}

/// Advances the signing frontier as far as predecessor linkage and UTXO
/// availability allow.
///
/// The frontier moves independently from finalization tracking: once an entry
/// is signed and has persisted reveal metadata, later entries may be signed
/// even if older ones are still waiting for confirmations.
async fn advance_forward_frontier<R: Reader + Signer + Wallet + Broadcaster>(
    state: &mut ChunkedEnvelopeWatcherState,
    ctx: Arc<ChunkedWriterContext<R>>,
    ops: &ChunkedEnvelopeOps,
    broadcast_handle: &L1BroadcastHandle,
) -> anyhow::Result<()> {
    while state.forward_frontier < state.next_db_idx {
        let idx = state.forward_frontier;
        let Some(entry) = ops.get_chunked_envelope_entry_async(idx).await? else {
            return Err(invalid_gap_error(idx).into());
        };

        if entry.status == ChunkedEnvelopeStatus::Finalized || entry_unlocks_successor(&entry) {
            state.forward_frontier += 1;
            continue;
        }

        if !matches!(
            entry.status,
            ChunkedEnvelopeStatus::Unsigned | ChunkedEnvelopeStatus::NeedsResign
        ) {
            state.forward_frontier += 1;
            continue;
        }

        debug!(idx, status = ?entry.status, "entry needs signing");
        let Some(prev_tail_wtxid) = resolve_prev_tail_wtxid(idx, ops).await? else {
            break;
        };

        match sign_chunked_envelope(idx, &entry, prev_tail_wtxid, broadcast_handle, ctx.clone())
            .await
        {
            Ok(updated) => {
                let signed_status = updated.status.clone();
                let reveal_refs = format_reveal_refs(&updated);
                debug!(
                    envelope_idx = idx,
                    commit_txid = %updated.commit_txid,
                    ?reveal_refs,
                    ?signed_status,
                    "entry signed successfully"
                );
                ops.put_chunked_envelope_entry_async(idx, updated.clone())
                    .await?;
                state.active_envelopes.insert(idx);

                if entry_unlocks_successor(&updated) {
                    state.forward_frontier += 1;
                    continue;
                }
            }
            Err(EnvelopeError::NotEnoughUtxos(need, have)) => {
                error!(idx, %need, %have, "waiting for sufficient utxos");
            }
            Err(e) => return Err(e.into()),
        }

        break;
    }

    Ok(())
}

/// Resolves the correct `prev_tail_wtxid` for the entry at index `curr`.
///
/// For index 0, returns [`Buf32::zero`] (first entry in chain). For all others,
/// returns the predecessor's persisted tail wtxid once the predecessor has been
/// signed. If the predecessor is still unsigned, the caller must wait.
async fn resolve_prev_tail_wtxid(
    curr: u64,
    ops: &ChunkedEnvelopeOps,
) -> anyhow::Result<Option<Buf32>> {
    if curr == 0 {
        return Ok(Some(Buf32::zero()));
    }

    let Some(prev_entry) = ops.get_chunked_envelope_entry_async(curr - 1).await? else {
        return Err(invalid_gap_error(curr - 1).into());
    };

    if !entry_unlocks_successor(&prev_entry) {
        return Ok(None);
    }

    Ok(Some(prev_entry.tail_wtxid()))
}

/// Checks commit tx status and broadcasts reveals once it is safe to do so.
///
/// Called when status is `Unpublished`. Returns:
/// - `CommitPublished` if commit is on-chain and reveals are broadcast and stored in DB
/// - `NeedsResign` if commit has invalid inputs or any reveal fails to broadcast
/// - `Unsigned` if commit is missing
/// - `Unpublished` if commit is still waiting
///
/// For single-reveal entries the reveal is broadcast as soon as the commit is
/// published (in mempool) — one reveal's ~99 KB vsize fits within Bitcoin
/// Core's default 101 KB descendant-size limit.
///
/// For multi-reveal entries we wait until the commit is **confirmed** (in a
/// block) before broadcasting. Multiple large reveals would otherwise exceed
/// the descendant-size limit and be rejected by the mempool.
async fn check_commit_and_broadcast_reveals(
    envelope_idx: u64,
    entry: &ChunkedEnvelopeEntry,
    bcast: &L1BroadcastHandle,
    client: &impl Broadcaster,
) -> anyhow::Result<ChunkedEnvelopeStatus> {
    let Some(commit) = bcast.get_tx_entry_by_id_async(entry.commit_txid).await? else {
        warn!(
            envelope_idx,
            commit_txid = %entry.commit_txid,
            "commit tx missing from broadcast db, will re-sign"
        );
        return Ok(ChunkedEnvelopeStatus::Unsigned);
    };

    // A single reveal fits within the default 101 KB mempool descendant-size
    // limit, so it can be broadcast as soon as the commit is in the mempool.
    // Multiple reveals would exceed that limit, so we wait for the commit to
    // be confirmed in a block first.
    let needs_commit_confirmed = entry.reveals.len() > 1;

    let ready = match commit.status {
        L1TxStatus::InvalidInputs => return Ok(ChunkedEnvelopeStatus::NeedsResign),
        L1TxStatus::Unpublished => false,
        L1TxStatus::Published => !needs_commit_confirmed,
        L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. } => true,
    };

    if !ready {
        return Ok(ChunkedEnvelopeStatus::Unpublished);
    }

    let reveal_refs = format_reveal_refs(entry);
    info!(
        envelope_idx,
        commit_txid = %entry.commit_txid,
        commit_status = ?commit.status,
        reveal_count = entry.reveals.len(),
        ?reveal_refs,
        "commit on-chain, broadcasting all reveals"
    );

    // Deserialize all reveal transactions.
    let mut reveal_txs = Vec::with_capacity(entry.reveals.len());
    for reveal in &entry.reveals {
        let tx: Transaction = btc_deserialize(&reveal.tx_bytes)
            .map_err(|e| anyhow::anyhow!("failed to deserialize reveal tx: {}", e))?;
        reveal_txs.push((reveal.txid, tx));
    }

    // Broadcast all reveals.
    for (txid, tx) in &reveal_txs {
        match client.send_raw_transaction(tx).await {
            Ok(_) => {
                debug!(
                    envelope_idx,
                    %txid,
                    "reveal tx broadcast successfully"
                );
            }
            Err(e)
                if e.is_missing_or_invalid_input() || matches!(e, ClientError::Server(-22, _)) =>
            {
                warn!(
                    envelope_idx,
                    %txid,
                    ?e,
                    "reveal tx has invalid inputs, will re-sign"
                );
                return Ok(ChunkedEnvelopeStatus::NeedsResign);
            }
            Err(e) => {
                // Could be "already in mempool" which is fine, or a network error.
                // We'll verify actual status on the next poll.
                warn!(
                    envelope_idx,
                    %txid,
                    ?e,
                    "broadcast returned error (may already be in mempool)"
                );
            }
        }
    }

    info!(
        envelope_idx,
        commit_txid = %entry.commit_txid,
        reveal_count = entry.reveals.len(),
        ?reveal_refs,
        "completed reveal broadcast attempt"
    );

    // Store all reveals in broadcast DB for tracking.
    for (txid, tx) in reveal_txs {
        let mut tx_entry = L1TxEntry::from_tx(&tx);
        tx_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(txid, tx_entry)
            .await
            .map_err(|e| anyhow::anyhow!("failed to store reveal tx: {}", e))?;
    }

    Ok(ChunkedEnvelopeStatus::CommitPublished)
}

/// Checks broadcast status of commit + all reveals (after reveals are in broadcast DB).
///
/// Called when status is `CommitPublished`, `Published`, or `Confirmed`.
/// The least-progressed transaction determines the overall envelope status.
async fn check_full_broadcast_status(
    envelope_idx: u64,
    entry: &ChunkedEnvelopeEntry,
    bcast: &L1BroadcastHandle,
) -> anyhow::Result<ChunkedEnvelopeStatus> {
    let Some(commit) = bcast.get_tx_entry_by_id_async(entry.commit_txid).await? else {
        warn!(
            envelope_idx,
            commit_txid = %entry.commit_txid,
            "commit tx missing from broadcast db, will re-sign"
        );
        return Ok(ChunkedEnvelopeStatus::Unsigned);
    };
    if commit.status == L1TxStatus::InvalidInputs {
        return Ok(ChunkedEnvelopeStatus::NeedsResign);
    }

    let mut min_progress = commit.status.clone();
    let mut reveal_l1_statuses = Vec::with_capacity(entry.reveals.len());
    for reveal in &entry.reveals {
        let Some(rtx) = bcast.get_tx_entry_by_id_async(reveal.txid).await? else {
            warn!(
                envelope_idx,
                txid = %reveal.txid,
                "reveal tx missing from broadcast db, will re-sign"
            );
            return Ok(ChunkedEnvelopeStatus::Unsigned);
        };
        if rtx.status == L1TxStatus::InvalidInputs {
            // This shouldn't happen if we waited for commit to be published first,
            // but handle it gracefully by re-signing.
            warn!(
                envelope_idx,
                txid = %reveal.txid,
                "reveal has InvalidInputs despite commit being published"
            );
            return Ok(ChunkedEnvelopeStatus::NeedsResign);
        }
        reveal_l1_statuses.push(format_tx_status(reveal.txid, &rtx.status));
        if is_less_progressed(&rtx.status, &min_progress) {
            min_progress = rtx.status;
        }
    }

    let envelope_status = to_envelope_status(&min_progress);
    if matches!(
        envelope_status,
        ChunkedEnvelopeStatus::Confirmed | ChunkedEnvelopeStatus::Finalized
    ) {
        let commit_l1_status = format_tx_status(entry.commit_txid, &commit.status);
        info!(
            envelope_idx,
            commit_txid = %entry.commit_txid,
            ?envelope_status,
            commit_l1_status = %commit_l1_status,
            ?reveal_l1_statuses,
            "chunked envelope advanced on L1"
        );
    }

    Ok(envelope_status)
}

/// Returns a progress ordinal for comparing [`L1TxStatus`] values.
///
/// Only used for ordering — never converted back into an enum.
/// `InvalidInputs` is excluded because the caller handles it via early return.
fn progress_ordinal(s: &L1TxStatus) -> u8 {
    match s {
        L1TxStatus::Unpublished => 0,
        L1TxStatus::Published => 1,
        L1TxStatus::Confirmed { .. } => 2,
        L1TxStatus::Finalized { .. } => 3,
        L1TxStatus::InvalidInputs => {
            unreachable!("InvalidInputs is handled before aggregation")
        }
    }
}

/// Returns `true` if `a` has made less broadcast progress than `b`.
fn is_less_progressed(a: &L1TxStatus, b: &L1TxStatus) -> bool {
    progress_ordinal(a) < progress_ordinal(b)
}

/// Maps a broadcast-layer [`L1TxStatus`] to the corresponding [`ChunkedEnvelopeStatus`].
///
/// Called after reveals are in broadcast DB (from `CommitPublished` state onwards).
/// Returns `CommitPublished` for `Unpublished` to avoid regressing the envelope status.
/// `InvalidInputs` is excluded — the caller must handle it separately since it
/// maps to [`ChunkedEnvelopeStatus::NeedsResign`], which has no `L1TxStatus`
/// counterpart.
fn to_envelope_status(s: &L1TxStatus) -> ChunkedEnvelopeStatus {
    match s {
        // Reveals may still be unpublished even though they're in broadcast DB.
        // Stay at CommitPublished until all are Published.
        L1TxStatus::Unpublished => ChunkedEnvelopeStatus::CommitPublished,
        L1TxStatus::Published => ChunkedEnvelopeStatus::Published,
        L1TxStatus::Confirmed { .. } => ChunkedEnvelopeStatus::Confirmed,
        L1TxStatus::Finalized { .. } => ChunkedEnvelopeStatus::Finalized,
        L1TxStatus::InvalidInputs => {
            unreachable!("InvalidInputs is handled before aggregation")
        }
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{
        absolute::LockTime, consensus::encode::serialize as btc_serialize, hashes::Hash,
        transaction::Version, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
        Witness,
    };
    use strata_db_types::types::RevealTxMeta;
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::{
        test_utils::{SendRawTransactionMode, TestBitcoinClient},
        writer::test_utils::{get_broadcast_handle, get_chunked_envelope_ops},
    };

    fn make_recovery_entry(
        status: ChunkedEnvelopeStatus,
        idx_tag: u8,
        signed: bool,
    ) -> ChunkedEnvelopeEntry {
        let mut entry = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![idx_tag; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        entry.status = status;
        if signed {
            entry.reveals = vec![RevealTxMeta {
                vout_index: 0,
                txid: Buf32::from([idx_tag; 32]),
                wtxid: Buf32::from([idx_tag.wrapping_add(1); 32]),
                tx_bytes: vec![idx_tag],
            }];
        }
        entry
    }

    #[test]
    fn test_recover_watcher_state_empty() {
        let ops = get_chunked_envelope_ops();
        let state = ChunkedEnvelopeWatcherState::recover(&ops).unwrap();
        assert_eq!(state.next_db_idx, 0);
        assert_eq!(state.forward_frontier, 0);
        assert!(state.active_envelopes.is_empty());
    }

    #[test]
    fn test_recover_watcher_state_tracks_active_entries_and_frontier() {
        let ops = get_chunked_envelope_ops();

        ops.put_chunked_envelope_entry_blocking(
            0,
            make_recovery_entry(ChunkedEnvelopeStatus::Finalized, 0x01, true),
        )
        .unwrap();
        ops.put_chunked_envelope_entry_blocking(
            1,
            make_recovery_entry(ChunkedEnvelopeStatus::Published, 0x02, true),
        )
        .unwrap();
        ops.put_chunked_envelope_entry_blocking(
            2,
            make_recovery_entry(ChunkedEnvelopeStatus::Unpublished, 0x03, true),
        )
        .unwrap();
        ops.put_chunked_envelope_entry_blocking(
            3,
            make_recovery_entry(ChunkedEnvelopeStatus::Unsigned, 0x04, false),
        )
        .unwrap();

        let state = ChunkedEnvelopeWatcherState::recover(&ops).unwrap();
        assert_eq!(state.next_db_idx, 4);
        assert_eq!(state.forward_frontier, 3);
        assert_eq!(state.active_envelopes, BTreeSet::from([1, 2, 3]));
    }

    #[test]
    fn test_recover_watcher_state_rejects_gap_before_tip() {
        let ops = get_chunked_envelope_ops();
        ops.put_chunked_envelope_entry_blocking(
            0,
            make_recovery_entry(ChunkedEnvelopeStatus::Finalized, 0x01, true),
        )
        .unwrap();
        ops.put_chunked_envelope_entry_blocking(
            2,
            make_recovery_entry(ChunkedEnvelopeStatus::Unsigned, 0x03, false),
        )
        .unwrap();

        let err = ChunkedEnvelopeWatcherState::recover(&ops).unwrap_err();
        let watcher_error = err
            .downcast_ref::<ChunkedEnvelopeWatcherError>()
            .expect("recovery should return a typed watcher error");
        assert!(matches!(
            watcher_error,
            ChunkedEnvelopeWatcherError::EntryGap { missing_idx: 1 }
        ));
    }

    #[test]
    fn test_progress_ordering_is_monotonic() {
        let statuses = [
            L1TxStatus::Unpublished,
            L1TxStatus::Published,
            L1TxStatus::Confirmed {
                confirmations: 1,
                block_hash: Buf32::zero(),
                block_height: 100,
            },
            L1TxStatus::Finalized {
                confirmations: 6,
                block_hash: Buf32::zero(),
                block_height: 100,
            },
        ];
        for window in statuses.windows(2) {
            assert!(
                is_less_progressed(&window[0], &window[1]),
                "{:?} should be less progressed than {:?}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn test_to_envelope_status_mapping() {
        // Unpublished maps to CommitPublished (to avoid regressing the envelope status
        // after reveals are stored in broadcast DB).
        assert_eq!(
            to_envelope_status(&L1TxStatus::Unpublished),
            ChunkedEnvelopeStatus::CommitPublished,
        );
        assert_eq!(
            to_envelope_status(&L1TxStatus::Published),
            ChunkedEnvelopeStatus::Published,
        );
        assert_eq!(
            to_envelope_status(&L1TxStatus::Confirmed {
                confirmations: 3,
                block_hash: Buf32::zero(),
                block_height: 100,
            }),
            ChunkedEnvelopeStatus::Confirmed,
        );
        assert_eq!(
            to_envelope_status(&L1TxStatus::Finalized {
                confirmations: 6,
                block_hash: Buf32::zero(),
                block_height: 100,
            }),
            ChunkedEnvelopeStatus::Finalized,
        );
    }

    #[test]
    fn test_least_progressed_determines_aggregate() {
        // All unpublished → CommitPublished (waiting for reveals to be published).
        assert_eq!(
            to_envelope_status(&L1TxStatus::Unpublished),
            ChunkedEnvelopeStatus::CommitPublished,
        );

        // All finalized → Finalized.
        assert_eq!(
            to_envelope_status(&L1TxStatus::Finalized {
                confirmations: 6,
                block_hash: Buf32::zero(),
                block_height: 100,
            }),
            ChunkedEnvelopeStatus::Finalized,
        );

        // One published, rest confirmed → published is least progressed.
        assert!(is_less_progressed(
            &L1TxStatus::Published,
            &L1TxStatus::Confirmed {
                confirmations: 3,
                block_hash: Buf32::zero(),
                block_height: 100,
            },
        ));
        assert_eq!(
            to_envelope_status(&L1TxStatus::Published),
            ChunkedEnvelopeStatus::Published,
        );
    }

    // Async state-machine tests for commit/reveal status transitions.

    /// Creates a minimal valid transaction for test database entries.
    fn make_test_tx() -> Transaction {
        Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![TxIn {
                previous_output: OutPoint {
                    txid: bitcoin::Txid::all_zeros(),
                    vout: 0,
                },
                script_sig: ScriptBuf::new(),
                witness: Witness::new(),
                sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            }],
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: ScriptBuf::new(),
            }],
        }
    }

    /// Creates a test entry with N reveal transactions containing valid tx_bytes.
    fn make_entry_with_reveals(n: usize) -> ChunkedEnvelopeEntry {
        let mut entry = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0xAA; 100]; n],
            MagicBytes::new([0x01, 0x02, 0x03, 0x04]),
        );
        entry.commit_txid = Buf32::from([0x11; 32]);
        entry.reveals = (0..n)
            .map(|i| {
                let tx = make_test_tx();
                RevealTxMeta {
                    vout_index: i as u32,
                    txid: Buf32::from([(0x20 + i as u8); 32]),
                    wtxid: Buf32::from([(0x30 + i as u8); 32]),
                    tx_bytes: btc_serialize(&tx),
                }
            })
            .collect();
        entry.status = ChunkedEnvelopeStatus::Unpublished;
        entry
    }

    #[tokio::test]
    async fn test_check_commit_unpublished_stays_waiting() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1);
        let entry = make_entry_with_reveals(2);

        // Store commit with Unpublished status — reveals should NOT be broadcast.
        let commit_entry = L1TxEntry::from_tx(&make_test_tx());
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(0, &entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Unpublished,
            "should stay Unpublished while commit is not yet published"
        );

        // Ensure reveals are not inserted in broadcast DB before commit is published.
        for reveal in &entry.reveals {
            let rtx = bcast.get_tx_entry_by_id_async(reveal.txid).await.unwrap();
            assert!(
                rtx.is_none(),
                "reveal should not be stored before commit publish"
            );
        }
    }

    #[tokio::test]
    async fn test_check_commit_missing_returns_unsigned() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1);
        let entry = make_entry_with_reveals(2);

        // Don't store commit at all — should return Unsigned for re-signing.
        let result = check_commit_and_broadcast_reveals(0, &entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Unsigned,
            "missing commit should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_check_commit_invalid_inputs_returns_needs_resign() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1);
        let entry = make_entry_with_reveals(2);

        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::InvalidInputs;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(0, &entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(result, ChunkedEnvelopeStatus::NeedsResign);
    }

    #[tokio::test]
    async fn test_check_commit_published_broadcasts_reveals() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1);
        let entry = make_entry_with_reveals(3);

        // Store commit as Confirmed (required for multi-reveal entries to pass the gate).
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Confirmed {
            confirmations: 1,
            block_hash: Buf32::from([0xBB; 32]),
            block_height: 100,
        };
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(0, &entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::CommitPublished,
            "should broadcast reveals and transition to CommitPublished"
        );

        // Verify all reveals were stored in broadcast DB with Published status.
        for reveal in &entry.reveals {
            let rtx = bcast
                .get_tx_entry_by_id_async(reveal.txid)
                .await
                .unwrap()
                .expect("reveal should be in broadcast DB");
            assert_eq!(
                rtx.status,
                L1TxStatus::Published,
                "reveal should be marked Published"
            );
        }
    }

    #[tokio::test]
    async fn test_check_commit_broadcast_missing_input_returns_needs_resign() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1)
            .with_send_raw_transaction_mode(SendRawTransactionMode::MissingOrInvalidInput);
        let entry = make_entry_with_reveals(2);

        // Store commit as Confirmed (required for multi-reveal entries to pass the gate).
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Confirmed {
            confirmations: 1,
            block_hash: Buf32::from([0xBB; 32]),
            block_height: 100,
        };
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(0, &entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::NeedsResign,
            "missing/invalid input during reveal broadcast should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_check_commit_broadcast_server_minus22_returns_needs_resign() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1)
            .with_send_raw_transaction_mode(SendRawTransactionMode::InvalidParameter);
        let entry = make_entry_with_reveals(2);

        // Store commit as Confirmed (required for multi-reveal entries to pass the gate).
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Confirmed {
            confirmations: 1,
            block_hash: Buf32::from([0xBB; 32]),
            block_height: 100,
        };
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(0, &entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::NeedsResign,
            "Server(-22, ..) during reveal broadcast should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_check_commit_broadcast_generic_error_keeps_commit_published_state() {
        let bcast = get_broadcast_handle();
        let client = TestBitcoinClient::new(1)
            .with_send_raw_transaction_mode(SendRawTransactionMode::GenericError);
        let entry = make_entry_with_reveals(2);

        // Store commit as Confirmed (required for multi-reveal entries to pass the gate).
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Confirmed {
            confirmations: 1,
            block_hash: Buf32::from([0xBB; 32]),
            block_height: 100,
        };
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_broadcast_reveals(0, &entry, &bcast, &client)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::CommitPublished,
            "generic reveal broadcast errors should still move to CommitPublished"
        );

        // Reveals are inserted for tracking even when RPC returned generic broadcast errors.
        for reveal in &entry.reveals {
            let rtx = bcast
                .get_tx_entry_by_id_async(reveal.txid)
                .await
                .unwrap()
                .expect("reveal should be in broadcast DB");
            assert_eq!(rtx.status, L1TxStatus::Published);
        }
    }

    #[tokio::test]
    async fn test_full_status_all_finalized() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        let finalized = L1TxStatus::Finalized {
            confirmations: 6,
            block_hash: Buf32::from([0xAA; 32]),
            block_height: 100,
        };

        // Store commit as Finalized.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = finalized.clone();
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Store all reveals as Finalized.
        for reveal in &entry.reveals {
            let mut rtx = L1TxEntry::from_tx(&make_test_tx());
            rtx.status = finalized.clone();
            bcast.put_tx_entry(reveal.txid, rtx).await.unwrap();
        }

        let result = check_full_broadcast_status(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(result, ChunkedEnvelopeStatus::Finalized);
    }

    #[tokio::test]
    async fn test_full_status_least_progressed_wins() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(3);

        let confirmed = L1TxStatus::Confirmed {
            confirmations: 3,
            block_hash: Buf32::from([0xBB; 32]),
            block_height: 100,
        };

        // Commit is Confirmed.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = confirmed.clone();
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Reveal 0: Confirmed.
        let mut r0 = L1TxEntry::from_tx(&make_test_tx());
        r0.status = confirmed.clone();
        bcast.put_tx_entry(entry.reveals[0].txid, r0).await.unwrap();

        // Reveal 1: Published (least progressed).
        let mut r1 = L1TxEntry::from_tx(&make_test_tx());
        r1.status = L1TxStatus::Published;
        bcast.put_tx_entry(entry.reveals[1].txid, r1).await.unwrap();

        // Reveal 2: Confirmed.
        let mut r2 = L1TxEntry::from_tx(&make_test_tx());
        r2.status = confirmed;
        bcast.put_tx_entry(entry.reveals[2].txid, r2).await.unwrap();

        let result = check_full_broadcast_status(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Published,
            "least progressed (Published) should determine overall status"
        );
    }

    #[tokio::test]
    async fn test_full_status_commit_missing_returns_unsigned() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        let result = check_full_broadcast_status(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Unsigned,
            "missing commit should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_full_status_reveal_missing_returns_unsigned() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Store commit.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Store only first reveal.
        let mut r0 = L1TxEntry::from_tx(&make_test_tx());
        r0.status = L1TxStatus::Published;
        bcast.put_tx_entry(entry.reveals[0].txid, r0).await.unwrap();

        // Second reveal is missing.
        let result = check_full_broadcast_status(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Unsigned,
            "missing reveal should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_full_status_reveal_invalid_inputs_returns_needs_resign() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Store commit as Published.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Reveal 0 is fine.
        let mut r0 = L1TxEntry::from_tx(&make_test_tx());
        r0.status = L1TxStatus::Published;
        bcast.put_tx_entry(entry.reveals[0].txid, r0).await.unwrap();

        // Reveal 1 has invalid inputs.
        let mut r1 = L1TxEntry::from_tx(&make_test_tx());
        r1.status = L1TxStatus::InvalidInputs;
        bcast.put_tx_entry(entry.reveals[1].txid, r1).await.unwrap();

        let result = check_full_broadcast_status(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::NeedsResign,
            "InvalidInputs on any reveal should trigger re-sign"
        );
    }

    #[tokio::test]
    async fn test_full_status_unpublished_maps_to_commit_published() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Commit is Published.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(entry.commit_txid, commit_entry)
            .await
            .unwrap();

        // Reveals are Unpublished (stored in DB but not yet in mempool).
        for reveal in &entry.reveals {
            let rtx = L1TxEntry::from_tx(&make_test_tx());
            // from_tx creates with Unpublished status by default
            bcast.put_tx_entry(reveal.txid, rtx).await.unwrap();
        }

        let result = check_full_broadcast_status(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::CommitPublished,
            "Unpublished L1TxStatus should map to CommitPublished to avoid status regression"
        );
    }

    #[tokio::test]
    async fn test_resolve_prev_tail_wtxid_from_signed_predecessor() {
        let ops = get_chunked_envelope_ops();

        // Entry[0]: simulate signed state (reveals populated with known wtxid).
        let mut e0 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x01; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        let real_tail = Buf32::from([0x42; 32]);
        e0.reveals = vec![RevealTxMeta {
            vout_index: 0,
            txid: Buf32::from([0x11; 32]),
            wtxid: real_tail,
            tx_bytes: vec![0xDE, 0xAD],
        }];
        e0.status = ChunkedEnvelopeStatus::Unpublished;
        ops.put_chunked_envelope_entry_async(0, e0).await.unwrap();

        // Entry[1]: prev_tail_wtxid is zero at creation (deferred to signing).
        let e1 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x02; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        ops.put_chunked_envelope_entry_async(1, e1).await.unwrap();

        // resolve_prev_tail_wtxid should return entry[0]'s real tail_wtxid.
        let resolved = resolve_prev_tail_wtxid(1, &ops).await.unwrap();
        assert_eq!(resolved, Some(real_tail));

        // Index 0 should return zero (first in chain).
        let resolved_zero = resolve_prev_tail_wtxid(0, &ops).await.unwrap();
        assert_eq!(resolved_zero, Some(Buf32::zero()));
    }

    #[tokio::test]
    async fn test_resolve_prev_tail_wtxid_waits_for_unsigned_predecessor() {
        let ops = get_chunked_envelope_ops();

        let e0 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x01; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        ops.put_chunked_envelope_entry_async(0, e0).await.unwrap();

        let e1 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x02; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        ops.put_chunked_envelope_entry_async(1, e1).await.unwrap();

        let resolved = resolve_prev_tail_wtxid(1, &ops).await.unwrap();
        assert_eq!(resolved, None);
    }

    #[tokio::test]
    async fn test_resolve_prev_tail_wtxid_missing_predecessor_errors() {
        let ops = get_chunked_envelope_ops();

        let e1 = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![0x02; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
        );
        ops.put_chunked_envelope_entry_async(1, e1).await.unwrap();

        let err = resolve_prev_tail_wtxid(1, &ops).await.unwrap_err();
        assert!(err.to_string().contains("gap at index 0"));
    }
}
