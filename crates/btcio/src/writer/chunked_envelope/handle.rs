//! Chunked envelope handle and lifecycle driver.
//!
//! Polls entries by sequential index and advances them through the status
//! lifecycle: Unsigned → Unpublished → CommitPublished → Published → Confirmed → Finalized.
//!
//! The `CommitPublished` intermediate state means the commit tx is accepted by
//! bitcoind or confirmed on L1, so successor envelopes may be signed. Reveal txs
//! are handed to the broadcaster only when doing so is mempool-policy safe.

use std::{collections::BTreeSet, future::Future, sync::Arc, time::Duration};

use bitcoin::{consensus::encode::deserialize as btc_deserialize, key::Keypair, Address};
use bitcoind_async_client::{
    error::ClientError,
    traits::{Broadcaster, Reader, Signer, Wallet},
    Client,
};
use strata_config::btcio::WriterConfig;
use strata_db_types::{
    chunked_envelope::{ChunkedEnvelopeEntry, ChunkedEnvelopeStatus, RevealTxMeta},
    common::L1TxId,
    l1_broadcast::{L1TxEntry, L1TxStatus},
};
use strata_primitives::buf::Buf32;
use strata_storage::ops::chunked_envelope::ChunkedEnvelopeOps;
use thiserror::Error;
use tokio::time::interval;
use tracing::*;

use super::{
    context::ChunkedWriterContext,
    signer::{sign_chunked_envelope, SignedChunkedEnvelope},
};
use crate::{
    broadcaster::{is_benign_minus25_message, L1BroadcastHandle},
    rpc_error::{is_retryable_envelope_error, retryable_reason},
    writer::builder::EnvelopeError,
    BtcioParams,
};

/// Maximum number of envelope rows to fetch per storage scan batch.
///
/// Recovery and tip-ingestion walk the DB in ordered chunks so they can
/// validate that indices are contiguous without materializing an arbitrarily
/// large range in a single call.
const ENTRY_SCAN_BATCH_SIZE: usize = 1_024;

/// Bitcoin Core v29's default `-limitdescendantsize`, in virtual bytes.
///
/// Source:
/// <https://github.com/bitcoin/bitcoin/blob/master/src/policy/policy.h#L74>
///
/// The policy includes the unconfirmed parent and all descendants. We only
/// enqueue a single reveal under an unconfirmed commit if the commit+reveal
/// package stays below this limit; otherwise reveals wait for commit
/// confirmation and then spend confirmed outputs independently.
const DEFAULT_DESCENDANT_SIZE_LIMIT_VBYTES: usize = 101_000;

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

    /// A commit tx disappeared after the envelope reached reveal tracking.
    #[error(
        "chunked envelope {envelope_idx} commit {commit_txid:?} missing from broadcast db in status {status:?}"
    )]
    CommitMissingAfterRevealTracking {
        envelope_idx: u64,
        commit_txid: L1TxId,
        status: ChunkedEnvelopeStatus,
    },
}

fn to_raw_buf32(txid: L1TxId) -> Buf32 {
    Buf32(txid.0)
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
    sequencer_keypair: Keypair,
    ops: Arc<ChunkedEnvelopeOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> anyhow::Result<(Arc<ChunkedEnvelopeHandle>, impl Future<Output = ()>)> {
    let watcher_state = ChunkedEnvelopeWatcherState::recover(ops.as_ref())?;
    let handle = Arc::new(ChunkedEnvelopeHandle::new(ops.clone()));

    let ctx = Arc::new(ChunkedWriterContext::new(
        btcio_params,
        config,
        sequencer_address,
        sequencer_keypair,
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
        .map(|reveal| format!("{:?}/{:?}", reveal.txid, reveal.wtxid))
        .collect()
}

fn format_tx_status(txid: L1TxId, status: &L1TxStatus) -> String {
    match status {
        L1TxStatus::Unpublished => format!("{txid:?}:unpublished"),
        L1TxStatus::Published => format!("{txid:?}:published"),
        L1TxStatus::InvalidInputs => format!("{txid:?}:invalid_inputs"),
        L1TxStatus::Confirmed {
            confirmations,
            block_hash,
            block_height,
        } => {
            format!("{txid:?}:confirmed@{block_height}/{block_hash} ({confirmations} confs)")
        }
        L1TxStatus::Finalized {
            confirmations,
            block_hash,
            block_height,
        } => {
            format!("{txid:?}:finalized@{block_height}/{block_hash} ({confirmations} confs)")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommitPublishResult {
    Published,
    InvalidInputs,
    Deferred(String),
}

/// Polls entries and drives them through signing, broadcast, and confirmation.
///
/// The lifecycle is:
/// 1. `Unsigned`/`NeedsResign` -> sign commit+reveals, store commit in broadcast DB ->
///    `Unpublished`.
/// 2. `Unpublished` -> wait for commit acceptance/rejection -> `CommitPublished` or `NeedsResign`.
/// 3. `CommitPublished` -> enqueue reveals when mempool-policy safe and wait for publication ->
///    `Published`.
/// 4. `Published` -> wait for confirmation -> `Confirmed`
/// 5. `Confirmed` -> wait for finalization -> `Finalized`
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
            reconcile_active_entries(&mut state, ops.as_ref(), &broadcast_handle).await?;
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

/// Returns `true` if this entry is in a state where successor entries can be
/// signed independently of it.
///
/// The watcher signs through one `forward_frontier`. This is not a global UTXO
/// reservation mechanism; it only prevents this watcher from signing the next
/// envelope before bitcoind has accepted or rejected the current commit tx.
/// This is kept as a named predicate for readability.
fn entry_unlocks_successor(entry: &ChunkedEnvelopeEntry) -> bool {
    matches!(
        entry.status,
        ChunkedEnvelopeStatus::CommitPublished
            | ChunkedEnvelopeStatus::Published
            | ChunkedEnvelopeStatus::Confirmed
            | ChunkedEnvelopeStatus::Finalized
    )
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
                check_commit_and_enqueue_reveals(idx, &entry, broadcast_handle).await?
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
                commit_txid = ?entry.commit_txid,
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

async fn publish_commit_immediately<R: Broadcaster>(
    client: &R,
    commit_tx_entry: &L1TxEntry,
) -> anyhow::Result<CommitPublishResult> {
    let tx = commit_tx_entry.try_to_tx()?;
    let txid = tx.compute_txid();
    debug!(%txid, vsize = tx.vsize(), "broadcasting chunked envelope commit");

    match client.send_raw_transaction(&tx).await {
        Ok(_) => {
            info!(%txid, "chunked envelope commit accepted by bitcoind");
            Ok(CommitPublishResult::Published)
        }
        Err(ClientError::Server(-25, msg)) => {
            // Bitcoind reuses -25 (RPC_VERIFY_ERROR) for both benign
            // already-accepted reasons and genuine rejections such as
            // `bad-txns-inputs-missingorspent`. Mapping the whole code to
            // Published would unlock successor signing and enqueue reveal
            // handling for a commit bitcoind never accepted.
            if is_benign_minus25_message(&msg) {
                info!(%txid, %msg, "chunked envelope commit already known or already in block");
                Ok(CommitPublishResult::Published)
            } else {
                warn!(%txid, %msg, "commit broadcast rejected by mempool (-25); will resign");
                Ok(CommitPublishResult::InvalidInputs)
            }
        }
        Err(ClientError::Server(-27, _)) => {
            info!(%txid, "chunked envelope commit already in chainstate");
            Ok(CommitPublishResult::Published)
        }
        Err(e) if e.is_rpc_verify_error() || matches!(e, ClientError::Server(-22, _)) => {
            warn!(%txid, ?e, "commit broadcast rejected by mempool; will resign");
            Ok(CommitPublishResult::InvalidInputs)
        }
        Err(e) if e.is_retriable() => Ok(CommitPublishResult::Deferred(e.to_string())),
        Err(e) => Err(e.into()),
    }
}

async fn persist_signed_envelope_and_publish_commit<R: Reader + Signer + Wallet + Broadcaster>(
    envelope_idx: u64,
    signed: SignedChunkedEnvelope,
    ctx: &ChunkedWriterContext<R>,
    ops: &ChunkedEnvelopeOps,
    broadcast_handle: &L1BroadcastHandle,
) -> anyhow::Result<ChunkedEnvelopeEntry> {
    let mut entry = signed.entry;
    let mut commit_tx_entry = signed.commit_tx_entry;

    // Persist the signed envelope before commit publication. If the process
    // stops after bitcoind accepts the commit, recovery still has the reveal
    // metadata needed to continue the envelope lifecycle.
    ops.put_chunked_envelope_entry_async(envelope_idx, entry.clone())
        .await?;
    let commit_broadcast_idx = broadcast_handle
        .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_tx_entry.clone())
        .await?;
    debug!(
        envelope_idx,
        commit_txid = ?entry.commit_txid,
        commit_wtxid = ?entry.commit_wtxid,
        reveal_count = entry.reveals.len(),
        "persisted signed chunked envelope before commit broadcast"
    );

    match publish_commit_immediately(ctx.client.as_ref(), &commit_tx_entry).await? {
        CommitPublishResult::Published => {
            commit_tx_entry.status = L1TxStatus::Published;
            broadcast_handle
                .put_tx_entry_by_idx(commit_broadcast_idx, commit_tx_entry)
                .await?;
            let commit = broadcast_handle
                .get_tx_entry_by_id_async(to_raw_buf32(entry.commit_txid))
                .await?
                .expect("commit tx was just stored");
            try_enqueue_reveals_if_policy_safe(envelope_idx, &entry, &commit, broadcast_handle)
                .await?;
            entry.status = ChunkedEnvelopeStatus::CommitPublished;
            ops.put_chunked_envelope_entry_async(envelope_idx, entry.clone())
                .await?;
            info!(
                envelope_idx,
                commit_txid = ?entry.commit_txid,
                reveal_count = entry.reveals.len(),
                "chunked envelope commit published"
            );
        }
        CommitPublishResult::InvalidInputs => {
            commit_tx_entry.status = L1TxStatus::InvalidInputs;
            broadcast_handle
                .put_tx_entry_by_idx(commit_broadcast_idx, commit_tx_entry)
                .await?;
            entry.status = ChunkedEnvelopeStatus::NeedsResign;
            ops.put_chunked_envelope_entry_async(envelope_idx, entry.clone())
                .await?;
            warn!(
                envelope_idx,
                commit_txid = ?entry.commit_txid,
                "chunked envelope commit has invalid inputs; entry needs resign"
            );
        }
        CommitPublishResult::Deferred(reason) => {
            // The broadcaster owns retrying the persisted commit tx. The
            // frontier stays on this envelope until reconciliation sees the
            // commit as Published or InvalidInputs.
            warn!(envelope_idx, %reason, "commit broadcast deferred; frontier blocked");
        }
    }

    Ok(entry)
}

/// Advances the signing frontier as far as commit publication and UTXO
/// availability allow.
///
/// The frontier moves independently from finalization tracking: once an entry's
/// commit is accepted by bitcoind or known invalid, later entries may be signed
/// even if older ones are still waiting for reveal broadcast or confirmations.
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
            debug!(idx, status = ?entry.status, "signing frontier blocked");
            break;
        }

        debug!(idx, status = ?entry.status, "entry needs signing");

        match sign_chunked_envelope(idx, &entry, ctx.clone()).await {
            Ok(signed) => {
                let updated = persist_signed_envelope_and_publish_commit(
                    idx,
                    signed,
                    ctx.as_ref(),
                    ops,
                    broadcast_handle,
                )
                .await?;
                let signed_status = updated.status.clone();
                let reveal_refs = format_reveal_refs(&updated);
                debug!(
                    envelope_idx = idx,
                    commit_txid = ?updated.commit_txid,
                    ?reveal_refs,
                    ?signed_status,
                    "entry signed successfully"
                );
                state.active_envelopes.insert(idx);

                if entry_unlocks_successor(&updated) {
                    state.forward_frontier += 1;
                    continue;
                }
            }
            Err(EnvelopeError::NotEnoughUtxos(need, have)) => {
                warn!(idx, %need, %have, "waiting for sufficient utxos");
            }
            Err(err) if is_retryable_envelope_error(&err) => {
                let reason = retryable_reason(&err);
                warn!(idx, %reason, "retrying chunked envelope signing after Bitcoin RPC error");
            }
            Err(e) => return Err(e.into()),
        }

        break;
    }

    Ok(())
}

/// Checks commit tx status and enqueues reveals once it is safe to do so.
///
/// Called when status is `Unpublished`. Returns:
/// - `CommitPublished` if commit is accepted by bitcoind or confirmed on L1
/// - `NeedsResign` if commit has invalid inputs
/// - `Unsigned` if commit is missing
/// - `Unpublished` if commit is still waiting
///
/// A published commit unlocks successor signing immediately. Reveal enqueueing
/// is separate: while the commit is unconfirmed, only a single reveal whose
/// commit+reveal package is below Bitcoin Core's default descendant-size limit
/// is enqueued. Other reveals wait until the commit confirms and then spend
/// confirmed outputs independently.
async fn check_commit_and_enqueue_reveals(
    envelope_idx: u64,
    entry: &ChunkedEnvelopeEntry,
    bcast: &L1BroadcastHandle,
) -> anyhow::Result<ChunkedEnvelopeStatus> {
    let Some(commit) = bcast
        .get_tx_entry_by_id_async(to_raw_buf32(entry.commit_txid))
        .await?
    else {
        warn!(
            envelope_idx,
            commit_txid = ?entry.commit_txid,
            "commit tx missing from broadcast db, will re-sign"
        );
        return Ok(ChunkedEnvelopeStatus::Unsigned);
    };

    match commit.status {
        L1TxStatus::InvalidInputs => {
            warn!(
                envelope_idx,
                commit_txid = ?entry.commit_txid,
                "chunked envelope commit has invalid inputs during status check"
            );
            return Ok(ChunkedEnvelopeStatus::NeedsResign);
        }
        L1TxStatus::Unpublished => {
            debug!(
                envelope_idx,
                commit_txid = ?entry.commit_txid,
                "chunked envelope commit still unpublished"
            );
            return Ok(ChunkedEnvelopeStatus::Unpublished);
        }
        L1TxStatus::Published | L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. } => {}
    }

    debug!(
        envelope_idx,
        commit_txid = ?entry.commit_txid,
        commit_status = ?commit.status,
        reveal_count = entry.reveals.len(),
        "chunked envelope commit publication observed"
    );
    try_enqueue_reveals_if_policy_safe(envelope_idx, entry, &commit, bcast).await?;

    Ok(ChunkedEnvelopeStatus::CommitPublished)
}

fn reveal_enqueue_is_policy_safe(
    entry: &ChunkedEnvelopeEntry,
    commit: &L1TxEntry,
) -> anyhow::Result<bool> {
    match commit.status {
        L1TxStatus::InvalidInputs | L1TxStatus::Unpublished => Ok(false),
        L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. } => Ok(true),
        L1TxStatus::Published => {
            let [reveal] = entry.reveals.as_slice() else {
                return Ok(false);
            };
            let commit_tx = commit.try_to_tx()?;
            let reveal_tx: bitcoin::Transaction = btc_deserialize(&reveal.tx_bytes)
                .map_err(|e| anyhow::anyhow!("failed to deserialize reveal tx: {}", e))?;
            let package_vsize = commit_tx.vsize() + reveal_tx.vsize();
            Ok(package_vsize < DEFAULT_DESCENDANT_SIZE_LIMIT_VBYTES)
        }
    }
}

async fn try_enqueue_reveals_if_policy_safe(
    envelope_idx: u64,
    entry: &ChunkedEnvelopeEntry,
    commit: &L1TxEntry,
    bcast: &L1BroadcastHandle,
) -> anyhow::Result<bool> {
    if !reveal_enqueue_is_policy_safe(entry, commit)? {
        debug!(
            envelope_idx,
            commit_txid = ?entry.commit_txid,
            commit_status = ?commit.status,
            reveal_count = entry.reveals.len(),
            "reveal enqueue waiting for commit confirmation or smaller package"
        );
        return Ok(false);
    }
    let reveal_refs = format_reveal_refs(entry);
    info!(
        envelope_idx,
        commit_txid = ?entry.commit_txid,
        commit_status = ?commit.status,
        reveal_count = entry.reveals.len(),
        ?reveal_refs,
        "commit ready, enqueueing all reveals"
    );

    for reveal in &entry.reveals {
        ensure_reveal_tx_entry(envelope_idx, reveal, bcast).await?;
    }

    info!(
        envelope_idx,
        commit_txid = ?entry.commit_txid,
        reveal_count = entry.reveals.len(),
        ?reveal_refs,
        "enqueued reveal transactions for broadcaster retry tracking"
    );

    Ok(true)
}

/// Ensures a reveal transaction is present in the broadcaster DB for retry tracking.
async fn ensure_reveal_tx_entry(
    envelope_idx: u64,
    reveal: &RevealTxMeta,
    bcast: &L1BroadcastHandle,
) -> anyhow::Result<()> {
    if bcast
        .get_tx_entry_by_id_async(to_raw_buf32(reveal.txid))
        .await?
        .is_some()
    {
        debug!(
            envelope_idx,
            txid = ?reveal.txid,
            "reveal tx already tracked by broadcaster"
        );
        return Ok(());
    }

    let tx = btc_deserialize(&reveal.tx_bytes)
        .map_err(|e| anyhow::anyhow!("failed to deserialize reveal tx: {}", e))?;
    bcast
        .put_tx_entry(to_raw_buf32(reveal.txid), L1TxEntry::from_tx(&tx))
        .await
        .map_err(|e| anyhow::anyhow!("failed to store reveal tx: {}", e))?;

    debug!(
        envelope_idx,
        txid = ?reveal.txid,
        "reveal tx enqueued in broadcaster"
    );
    Ok(())
}

/// Checks broadcast status of commit + reveals.
///
/// Called when status is `CommitPublished`, `Published`, or `Confirmed`.
/// Missing reveals are enqueued only when policy-safe. Once all reveal txs are
/// in the broadcast DB, the least-progressed tx determines the envelope status.
async fn check_full_broadcast_status(
    envelope_idx: u64,
    entry: &ChunkedEnvelopeEntry,
    bcast: &L1BroadcastHandle,
) -> anyhow::Result<ChunkedEnvelopeStatus> {
    let Some(commit) = bcast
        .get_tx_entry_by_id_async(to_raw_buf32(entry.commit_txid))
        .await?
    else {
        error!(
            envelope_idx,
            commit_txid = ?entry.commit_txid,
            status = ?entry.status,
            "commit tx missing from broadcast db after reveals were expected to be tracked"
        );
        return Err(
            ChunkedEnvelopeWatcherError::CommitMissingAfterRevealTracking {
                envelope_idx,
                commit_txid: entry.commit_txid,
                status: entry.status.clone(),
            }
            .into(),
        );
    };
    if commit.status == L1TxStatus::InvalidInputs {
        return Ok(ChunkedEnvelopeStatus::NeedsResign);
    }

    let can_enqueue_missing_reveals = reveal_enqueue_is_policy_safe(entry, &commit)?;
    let mut min_progress = commit.status.clone();
    let mut reveal_l1_statuses = Vec::with_capacity(entry.reveals.len());
    for reveal in &entry.reveals {
        let Some(rtx) = bcast
            .get_tx_entry_by_id_async(to_raw_buf32(reveal.txid))
            .await?
        else {
            if !can_enqueue_missing_reveals {
                debug!(
                    envelope_idx,
                    txid = ?reveal.txid,
                    commit_status = ?commit.status,
                    "reveal tx not enqueued yet; waiting for commit confirmation or smaller package"
                );
                return Ok(ChunkedEnvelopeStatus::CommitPublished);
            }
            warn!(
                envelope_idx,
                txid = ?reveal.txid,
                "reveal tx missing from broadcast db, restoring from persisted reveal bytes"
            );
            ensure_reveal_tx_entry(envelope_idx, reveal, bcast).await?;
            min_progress = L1TxStatus::Unpublished;
            reveal_l1_statuses.push(format_tx_status(reveal.txid, &min_progress));
            continue;
        };
        if rtx.status == L1TxStatus::InvalidInputs {
            // This shouldn't happen if we waited for commit to be published first,
            // but handle it gracefully by re-signing.
            warn!(
                envelope_idx,
                txid = ?reveal.txid,
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
            commit_txid = ?entry.commit_txid,
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
    use std::collections::BTreeSet;

    use bitcoin::{
        absolute::LockTime, consensus::encode::serialize as btc_serialize, hashes::Hash,
        transaction::Version, Amount, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
        Witness,
    };
    use strata_db_types::{
        chunked_envelope::RevealTxMeta,
        common::{L1TxId, L1WtxId},
    };
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::{
        test_utils::{SendRawTransactionMode, TestBitcoinClient},
        writer::test_utils::{get_broadcast_handle, get_chunked_envelope_ops},
    };

    fn bytes_from_start(start: u8) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        for (idx, byte) in bytes.iter_mut().enumerate() {
            *byte = start.wrapping_add(idx as u8);
        }
        bytes
    }

    fn reversed_hex(bytes: [u8; 32]) -> String {
        bytes
            .into_iter()
            .rev()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn make_recovery_entry(
        status: ChunkedEnvelopeStatus,
        idx_tag: u8,
        signed: bool,
    ) -> ChunkedEnvelopeEntry {
        let mut entry = ChunkedEnvelopeEntry::new_unsigned(
            vec![vec![idx_tag; 50]],
            MagicBytes::new([0xAA, 0xBB, 0xCC, 0xDD]),
            1,
        );
        entry.status = status;
        if signed {
            entry.reveals = vec![RevealTxMeta {
                vout_index: 0,
                txid: L1TxId::from([idx_tag; 32]),
                wtxid: L1WtxId::from([idx_tag.wrapping_add(1); 32]),
                tx_bytes: vec![idx_tag],
            }];
        }
        entry
    }

    #[test]
    fn format_tx_status_uses_full_reversed_txid() {
        let txid_bytes = bytes_from_start(0x10);
        let status = L1TxStatus::Published;

        assert_eq!(
            format_tx_status(L1TxId::from(txid_bytes), &status),
            format!("{}:published", reversed_hex(txid_bytes))
        );
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
        assert_eq!(state.forward_frontier, 2);
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
            1,
        );
        entry.commit_txid = L1TxId::from([0x11; 32]);
        entry.reveals = (0..n)
            .map(|i| {
                let tx = make_test_tx();
                RevealTxMeta {
                    vout_index: i as u32,
                    txid: L1TxId::from([(0x20 + i as u8); 32]),
                    wtxid: L1WtxId::from([(0x30 + i as u8); 32]),
                    tx_bytes: btc_serialize(&tx),
                }
            })
            .collect();
        entry.status = ChunkedEnvelopeStatus::Unpublished;
        entry
    }

    #[tokio::test]
    async fn test_publish_commit_immediately_defers_retriable_rpc_errors() {
        let client = TestBitcoinClient::new(0)
            .with_send_raw_transaction_mode(SendRawTransactionMode::ConnectionError);
        let commit_tx_entry = L1TxEntry::from_tx(&make_test_tx());

        let result = publish_commit_immediately(&client, &commit_tx_entry)
            .await
            .unwrap();

        assert!(matches!(
            result,
            CommitPublishResult::Deferred(reason) if reason.contains("connection refused")
        ));
    }

    #[tokio::test]
    async fn test_check_commit_unpublished_stays_waiting() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Store commit with Unpublished status — reveals should NOT be broadcast.
        let commit_entry = L1TxEntry::from_tx(&make_test_tx());
        bcast
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_enqueue_reveals(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::Unpublished,
            "should stay Unpublished while commit is not yet published"
        );

        // Ensure reveals are not inserted in broadcast DB before commit is published.
        for reveal in &entry.reveals {
            let rtx = bcast
                .get_tx_entry_by_id_async(to_raw_buf32(reveal.txid))
                .await
                .unwrap();
            assert!(
                rtx.is_none(),
                "reveal should not be stored before commit publish"
            );
        }
    }

    #[tokio::test]
    async fn test_check_commit_missing_returns_unsigned() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Don't store commit at all — should return Unsigned for re-signing.
        let result = check_commit_and_enqueue_reveals(0, &entry, &bcast)
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
        let entry = make_entry_with_reveals(2);

        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::InvalidInputs;
        bcast
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_enqueue_reveals(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(result, ChunkedEnvelopeStatus::NeedsResign);
    }

    #[tokio::test]
    async fn test_check_commit_confirmed_enqueues_multi_reveals_as_unpublished() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(3);

        // Store commit as Confirmed (required for multi-reveal entries to pass the gate).
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Confirmed {
            confirmations: 1,
            block_hash: Buf32::from([0xBB; 32]),
            block_height: 100,
        };
        bcast
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_enqueue_reveals(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::CommitPublished,
            "should enqueue reveals and transition to CommitPublished"
        );

        // Reveals enter the broadcaster as Unpublished so its retry loop owns sendrawtransaction.
        for reveal in &entry.reveals {
            let rtx = bcast
                .get_tx_entry_by_id_async(to_raw_buf32(reveal.txid))
                .await
                .unwrap()
                .expect("reveal should be in broadcast DB");
            assert_eq!(
                rtx.status,
                L1TxStatus::Unpublished,
                "reveal should be queued for broadcaster retry"
            );
        }
    }

    #[tokio::test]
    async fn test_check_commit_published_enqueues_single_reveal_under_descendant_limit() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(1);

        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_enqueue_reveals(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(result, ChunkedEnvelopeStatus::CommitPublished);

        let rtx = bcast
            .get_tx_entry_by_id_async(to_raw_buf32(entry.reveals[0].txid))
            .await
            .unwrap()
            .expect("single reveal should be queued under the descendant-size limit");
        assert_eq!(rtx.status, L1TxStatus::Unpublished);
    }

    #[tokio::test]
    async fn test_check_commit_published_waits_on_multi_reveal_confirmation() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        let result = check_commit_and_enqueue_reveals(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(result, ChunkedEnvelopeStatus::CommitPublished);

        for reveal in &entry.reveals {
            let rtx = bcast
                .get_tx_entry_by_id_async(to_raw_buf32(reveal.txid))
                .await
                .unwrap();
            assert!(
                rtx.is_none(),
                "multi-reveal tx should wait until commit confirmation"
            );
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
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        // Store all reveals as Finalized.
        for reveal in &entry.reveals {
            let mut rtx = L1TxEntry::from_tx(&make_test_tx());
            rtx.status = finalized.clone();
            bcast
                .put_tx_entry(to_raw_buf32(reveal.txid), rtx)
                .await
                .unwrap();
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
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        // Reveal 0: Confirmed.
        let mut r0 = L1TxEntry::from_tx(&make_test_tx());
        r0.status = confirmed.clone();
        bcast
            .put_tx_entry(to_raw_buf32(entry.reveals[0].txid), r0)
            .await
            .unwrap();

        // Reveal 1: Published (least progressed).
        let mut r1 = L1TxEntry::from_tx(&make_test_tx());
        r1.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(to_raw_buf32(entry.reveals[1].txid), r1)
            .await
            .unwrap();

        // Reveal 2: Confirmed.
        let mut r2 = L1TxEntry::from_tx(&make_test_tx());
        r2.status = confirmed;
        bcast
            .put_tx_entry(to_raw_buf32(entry.reveals[2].txid), r2)
            .await
            .unwrap();

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
    async fn test_full_status_commit_missing_errors_without_resign() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        let err = check_full_broadcast_status(0, &entry, &bcast)
            .await
            .unwrap_err();
        assert!(matches!(
            err.downcast_ref::<ChunkedEnvelopeWatcherError>(),
            Some(ChunkedEnvelopeWatcherError::CommitMissingAfterRevealTracking {
                envelope_idx: 0,
                commit_txid,
                status: ChunkedEnvelopeStatus::Unpublished,
            }) if *commit_txid == entry.commit_txid
        ));
    }

    #[tokio::test]
    async fn test_full_status_reveal_missing_requeues_without_resign() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Store commit as confirmed so missing multi-reveals are policy-safe to restore.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Confirmed {
            confirmations: 1,
            block_hash: Buf32::from([0xBB; 32]),
            block_height: 100,
        };
        bcast
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        // Store only first reveal.
        let mut r0 = L1TxEntry::from_tx(&make_test_tx());
        r0.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(to_raw_buf32(entry.reveals[0].txid), r0)
            .await
            .unwrap();

        // Second reveal is missing.
        let result = check_full_broadcast_status(0, &entry, &bcast)
            .await
            .unwrap();
        assert_eq!(
            result,
            ChunkedEnvelopeStatus::CommitPublished,
            "missing reveal should be restored without rebuilding the commit"
        );

        let restored = bcast
            .get_tx_entry_by_id_async(to_raw_buf32(entry.reveals[1].txid))
            .await
            .unwrap()
            .expect("missing reveal should be restored in broadcast DB");
        assert_eq!(restored.status, L1TxStatus::Unpublished);
    }

    #[tokio::test]
    async fn test_full_status_reveal_invalid_inputs_returns_needs_resign() {
        let bcast = get_broadcast_handle();
        let entry = make_entry_with_reveals(2);

        // Store commit as Published.
        let mut commit_entry = L1TxEntry::from_tx(&make_test_tx());
        commit_entry.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        // Reveal 0 is fine.
        let mut r0 = L1TxEntry::from_tx(&make_test_tx());
        r0.status = L1TxStatus::Published;
        bcast
            .put_tx_entry(to_raw_buf32(entry.reveals[0].txid), r0)
            .await
            .unwrap();

        // Reveal 1 has invalid inputs.
        let mut r1 = L1TxEntry::from_tx(&make_test_tx());
        r1.status = L1TxStatus::InvalidInputs;
        bcast
            .put_tx_entry(to_raw_buf32(entry.reveals[1].txid), r1)
            .await
            .unwrap();

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
            .put_tx_entry(to_raw_buf32(entry.commit_txid), commit_entry)
            .await
            .unwrap();

        // Reveals are Unpublished (stored in DB but not yet in mempool).
        for reveal in &entry.reveals {
            let rtx = L1TxEntry::from_tx(&make_test_tx());
            // from_tx creates with Unpublished status by default
            bcast
                .put_tx_entry(to_raw_buf32(reveal.txid), rtx)
                .await
                .unwrap();
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

    // Cross-batch independence is exercised by functional tests.
}
