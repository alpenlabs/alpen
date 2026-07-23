//! Cancels queued checkpoint submissions left over from a previous run.
//!
//! A sequencer that restarts with signed-but-unpublished checkpoints in the writer
//! queue republishes them: the broadcaster resumes from its database and the watcher
//! drains the writer queue, both before anything re-checks whether ASM has accepted
//! those epochs in the meantime. This pass retires the entries that are no longer
//! wanted, and it has to run during startup, before either service starts, because a
//! transaction that has been broadcast cannot be recalled.
//!
//! That constraint also decides what is left alone. Published bundles stay, and so do
//! bundles whose commit or reveal escaped before the crash: those are relinked to the
//! in-flight envelope and marked retiring, so the watcher tracks the original but
//! abandons it instead of re-signing if the envelope later fails. Relinking matches on
//! the checkpoint's identity rather than its epoch, because a fork or a rebuild can
//! leave several candidates for one epoch in flight and they are not interchangeable.
//!
//! This lives above [`strata_btcio`] rather than inside it. The queues belong to btcio,
//! but working out which checkpoint a queued payload holds, and whether it is still
//! worth publishing, is checkpoint knowledge.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use bitcoin::{Transaction, consensus::deserialize};
use strata_btc_types::TxidExt;
use strata_btcio::writer::PayloadCheckpointRef;
use strata_db_types::{
    DbResult,
    backend::DatabaseBackend,
    common::L1TxId,
    l1_broadcast::{L1TxEntry, L1TxStatus},
    l1_writer::{BundledPayloadEntry, IntentEntry, IntentStatus, L1BundleStatus},
};
use strata_identifiers::{Buf32, Epoch};
use strata_l1_txfmt::MagicBytes;
use strata_storage::{BroadcastDbOps, NodeStorage};
use tracing::{debug, warn};

use crate::l1_tx::{checkpoint_from_tx, inspect_l1_payload};

/// Cancels queued checkpoint submissions for epochs declared final by the client.
///
/// Safe for any node to run: declared-final epochs cannot roll back, so a queued
/// duplicate can only waste fees. This is also what clears a duplicate backlog
/// accumulated before a node upgraded to this code. If the client has not declared an
/// epoch final yet, this pass leaves the queue untouched.
pub fn cancel_settled_checkpoint_submissions(
    storage: &NodeStorage,
    last_settled_epoch: Option<Epoch>,
    magic_bytes: [u8; 4],
) -> Result<WriterCancelStats> {
    let Some(last_settled_epoch) = last_settled_epoch else {
        return Ok(WriterCancelStats::default());
    };

    cancel_checkpoint_submissions_for_epoch_side(
        &StorageCancelContext::new(storage),
        last_settled_epoch,
        magic_bytes,
        CheckpointEpochSide::Settled,
    )
}

/// Cancels queued checkpoint submissions for epochs ASM has not verified yet.
///
/// Only safe where something will rebuild them. Cancelling an unaccepted epoch
/// removes that epoch's only in-flight submission, so the caller must be deleting the
/// local artifacts that trigger a rebuild.
pub fn cancel_unaccepted_checkpoint_submissions(
    storage: &NodeStorage,
    first_unaccepted_epoch: Epoch,
    magic_bytes: [u8; 4],
) -> Result<WriterCancelStats> {
    cancel_checkpoint_submissions_for_epoch_side(
        &StorageCancelContext::new(storage),
        first_unaccepted_epoch,
        magic_bytes,
        CheckpointEpochSide::Unaccepted,
    )
}

/// Cancels declared-final and unaccepted queued checkpoint submissions.
///
/// Checkpoints between `last_settled_epoch` and `first_unaccepted_epoch` are verified
/// but reorgable, so this leaves them in flight.
pub fn cancel_queued_checkpoint_submissions(
    storage: &NodeStorage,
    last_settled_epoch: Option<Epoch>,
    first_unaccepted_epoch: Epoch,
    magic_bytes: [u8; 4],
) -> Result<WriterCancelStats> {
    let mut stats =
        cancel_settled_checkpoint_submissions(storage, last_settled_epoch, magic_bytes)?;
    stats.merge(cancel_unaccepted_checkpoint_submissions(
        storage,
        first_unaccepted_epoch,
        magic_bytes,
    )?);
    Ok(stats)
}

/// What a cancellation pass changed, for the caller's startup log line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WriterCancelStats {
    /// Intents moved to [`IntentStatus::Abandoned`].
    pub abandoned_intents: usize,
    /// Bundles moved to [`L1BundleStatus::Abandoned`].
    pub abandoned_bundles: usize,
    /// Bundles left alone because one of their transactions reached the network.
    pub left_published_bundles: usize,
    /// Bundles pointed at an escaped envelope instead of being abandoned.
    pub relinked_bundles: usize,
    /// Broadcaster entries moved to [`L1TxStatus::InvalidInputs`].
    pub invalidated_txs: usize,
    /// Intents abandoned because the bundle they referenced was missing.
    pub repaired_orphans: usize,
}

impl WriterCancelStats {
    /// Adds another pass's totals into this one.
    pub fn merge(&mut self, other: Self) {
        self.abandoned_intents += other.abandoned_intents;
        self.abandoned_bundles += other.abandoned_bundles;
        self.left_published_bundles += other.left_published_bundles;
        self.relinked_bundles += other.relinked_bundles;
        self.invalidated_txs += other.invalidated_txs;
        self.repaired_orphans += other.repaired_orphans;
    }
}

/// Storage the cancellation pass reads and writes.
///
/// The pass runs before the writer and broadcaster services exist, so it cannot go
/// through their handles. This trait is the seam that keeps it off concrete databases
/// anyway, in the same spirit as btcio's own watcher service context.
pub(crate) trait CheckpointCancelContext {
    /// Returns the index the next bundle will be written to.
    fn next_payload_idx(&self) -> DbResult<u64>;

    /// Returns the bundle at `idx`.
    fn payload_entry(&self, idx: u64) -> DbResult<Option<BundledPayloadEntry>>;

    /// Overwrites the bundle at `idx`.
    fn put_payload_entry(&self, idx: u64, entry: BundledPayloadEntry) -> DbResult<()>;

    /// Returns the index the next intent will be written to.
    fn next_intent_idx(&self) -> DbResult<u64>;

    /// Returns the intent at `idx`.
    fn intent_entry(&self, idx: u64) -> DbResult<Option<IntentEntry>>;

    /// Overwrites the intent stored under `intent_id`.
    fn update_intent_entry(&self, intent_id: Buf32, entry: IntentEntry) -> DbResult<()>;

    /// Returns the index the next broadcaster entry will be written to.
    fn next_broadcast_idx(&self) -> DbResult<u64>;

    /// Returns the transaction id at broadcaster index `idx`.
    fn broadcast_txid(&self, idx: u64) -> DbResult<Option<Buf32>>;

    /// Returns the broadcaster entry at `idx`.
    fn broadcast_entry(&self, idx: u64) -> DbResult<Option<L1TxEntry>>;

    /// Returns the broadcaster entry for `txid`.
    fn broadcast_entry_by_id(&self, txid: Buf32) -> DbResult<Option<L1TxEntry>>;

    /// Overwrites the broadcaster entry for `txid`.
    fn put_broadcast_entry(&self, txid: Buf32, entry: L1TxEntry) -> DbResult<()>;

    /// Overwrites the broadcaster entry at `idx`.
    fn put_broadcast_entry_by_idx(&self, idx: u64, entry: L1TxEntry) -> DbResult<()>;
}

/// [`CheckpointCancelContext`] backed by the node's storage layer.
struct StorageCancelContext<'s> {
    storage: &'s NodeStorage,
    broadcast: BroadcastDbOps,
}

impl<'s> StorageCancelContext<'s> {
    fn new(storage: &'s NodeStorage) -> Self {
        let broadcast = BroadcastDbOps::new(storage.handle().clone(), storage.db().broadcast_db());
        Self { storage, broadcast }
    }
}

impl CheckpointCancelContext for StorageCancelContext<'_> {
    fn next_payload_idx(&self) -> DbResult<u64> {
        self.storage.l1_writer().get_next_payload_idx_blocking()
    }

    fn payload_entry(&self, idx: u64) -> DbResult<Option<BundledPayloadEntry>> {
        self.storage
            .l1_writer()
            .get_payload_entry_by_idx_blocking(idx)
    }

    fn put_payload_entry(&self, idx: u64, entry: BundledPayloadEntry) -> DbResult<()> {
        self.storage
            .l1_writer()
            .put_payload_entry_blocking(idx, entry)
    }

    fn next_intent_idx(&self) -> DbResult<u64> {
        self.storage.l1_writer().get_next_intent_idx_blocking()
    }

    fn intent_entry(&self, idx: u64) -> DbResult<Option<IntentEntry>> {
        self.storage.l1_writer().get_intent_by_idx_blocking(idx)
    }

    fn update_intent_entry(&self, intent_id: Buf32, entry: IntentEntry) -> DbResult<()> {
        self.storage
            .l1_writer()
            .update_intent_entry_blocking(intent_id, entry)
    }

    fn next_broadcast_idx(&self) -> DbResult<u64> {
        self.broadcast.get_next_tx_idx_blocking()
    }

    fn broadcast_txid(&self, idx: u64) -> DbResult<Option<Buf32>> {
        self.broadcast.get_txid_blocking(idx)
    }

    fn broadcast_entry(&self, idx: u64) -> DbResult<Option<L1TxEntry>> {
        self.broadcast.get_tx_entry_blocking(idx)
    }

    fn broadcast_entry_by_id(&self, txid: Buf32) -> DbResult<Option<L1TxEntry>> {
        self.broadcast.get_tx_entry_by_id_blocking(txid)
    }

    fn put_broadcast_entry(&self, txid: Buf32, entry: L1TxEntry) -> DbResult<()> {
        self.broadcast
            .put_tx_entry_blocking(txid, entry)
            .map(|_| ())
    }

    fn put_broadcast_entry_by_idx(&self, idx: u64, entry: L1TxEntry) -> DbResult<()> {
        self.broadcast.put_tx_entry_by_idx_blocking(idx, entry)
    }
}

/// Which cancellable epoch range a queued checkpoint sits in.
///
/// The ranges get different treatment, so a pass handles one at a time: declared-final
/// epochs are cancelled unconditionally, while unaccepted ones are only cancelled when
/// the caller is also deleting the artifacts that drive a rebuild. Verified but
/// non-final epochs fall between these ranges and remain untouched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckpointEpochSide {
    /// Epochs at or below the client-declared final tip.
    Settled,
    /// Epochs above the ASM verified tip.
    Unaccepted,
}

impl CheckpointEpochSide {
    /// Returns whether `epoch` falls in the range selected by `boundary_epoch`.
    fn contains(self, epoch: Epoch, boundary_epoch: Epoch) -> bool {
        match self {
            Self::Settled => epoch <= boundary_epoch,
            Self::Unaccepted => epoch >= boundary_epoch,
        }
    }

    /// Returns whether a bundle on this side may be relinked to an escaped envelope.
    ///
    /// Only unaccepted epochs can be: a settled epoch is never rebuilt, so there is no
    /// second submission for the escaped original to displace.
    fn allows_relink(self) -> bool {
        self == Self::Unaccepted
    }

    /// Returns whether a cancelled checkpoint on this side is rebuilt afterwards.
    ///
    /// Only unaccepted epochs are: the caller deletes their local artifacts so the
    /// checkpoint worker builds them again. Settled epochs are already accepted, so
    /// nothing rebuilds them and their intents stay as they are.
    fn rebuilds_checkpoints(self) -> bool {
        self == Self::Unaccepted
    }
}

/// What to do with a queued bundle, given how far it got before the restart.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriterCancelAction {
    /// Retire the bundle; nothing was broadcast for it.
    Cancel,
    /// Retire the bundle and invalidate the transactions it had queued.
    CancelAndInvalidate,
    /// Leave it: it reached the network and cannot be recalled.
    Leave,
}

/// Maps a bundle status onto its cancellation action.
fn plan_writer_cancellation(status: &L1BundleStatus) -> WriterCancelAction {
    match status {
        L1BundleStatus::Unsigned
        | L1BundleStatus::PendingRevealTxSign(_)
        | L1BundleStatus::NeedsResign => WriterCancelAction::Cancel,
        L1BundleStatus::Unpublished | L1BundleStatus::Abandoned | L1BundleStatus::Retiring => {
            WriterCancelAction::CancelAndInvalidate
        }
        L1BundleStatus::Published | L1BundleStatus::Confirmed | L1BundleStatus::Finalized => {
            WriterCancelAction::Leave
        }
    }
}

/// How far an escaped envelope got, ordered so the furthest along wins.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum EscapeProgress {
    /// Only the commit reached the network; the reveal is still sendable.
    CommitOnly,
    /// The reveal was broadcast but is not in a block yet.
    Published,
    /// The reveal is in a block.
    Confirmed,
    /// The reveal is buried deep enough to be treated as final.
    Finalized,
}

impl EscapeProgress {
    /// Maps a broadcaster status onto its progress, for an entry already known to
    /// have escaped.
    fn from_status(status: &L1TxStatus) -> Option<Self> {
        match status {
            L1TxStatus::Published => Some(Self::Published),
            L1TxStatus::Confirmed { .. } => Some(Self::Confirmed),
            L1TxStatus::Finalized { .. } => Some(Self::Finalized),
            _ => None,
        }
    }
}

/// The commit/reveal pair of an envelope that reached the network before a crash.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EscapedEnvelopeTxs {
    /// Transaction funding the reveal, resolved from the reveal's prevout.
    commit_txid: Buf32,
    /// Transaction carrying the checkpoint envelope.
    reveal_txid: Buf32,
    /// Epoch of the checkpoint inside the envelope, kept for the log lines.
    epoch: Epoch,
    /// How far this envelope got, used to pick between duplicates.
    progress: EscapeProgress,
}

/// Escaped envelopes found during the broadcaster sweep, keyed by checkpoint identity.
///
/// Keyed by [`crate::l1_tx::checkpoint_payload_id`] rather than by epoch. A fork or a
/// rebuild can leave several candidates for one epoch in the queue at once, and
/// relinking a queued bundle to a different candidate's transaction would have the
/// watcher finalize the bundle against a checkpoint that is not the one it holds.
type EscapedCheckpointTxs = BTreeMap<Buf32, EscapedEnvelopeTxs>;

/// Cancels the queued checkpoints in one cancellable epoch range.
///
/// The broadcaster sweep runs first so the writer pass knows which checkpoints already
/// have transactions in flight and can relink to them instead of abandoning their
/// bundles.
fn cancel_checkpoint_submissions_for_epoch_side(
    ctx: &impl CheckpointCancelContext,
    boundary_epoch: Epoch,
    magic_bytes: [u8; 4],
    epoch_side: CheckpointEpochSide,
) -> Result<WriterCancelStats> {
    let mut stats = WriterCancelStats::default();
    let (invalidated_txs, escaped_checkpoint_txs) =
        invalidate_checkpoint_txs_by_decoding(ctx, boundary_epoch, magic_bytes, epoch_side)?;
    stats.invalidated_txs += invalidated_txs;

    let next_payload_idx = ctx
        .next_payload_idx()
        .context("read next L1 writer payload index")?;
    let first_unfinalized_payload_idx = first_payload_after_last_finalized(ctx, next_payload_idx)?;

    for payload_idx in first_unfinalized_payload_idx..next_payload_idx {
        let Some(mut entry) = ctx
            .payload_entry(payload_idx)
            .with_context(|| format!("read L1 writer payload entry {payload_idx}"))?
        else {
            continue;
        };
        let PayloadCheckpointRef::Checkpoint { epoch, id } = inspect_l1_payload(&entry.payload)
        else {
            continue;
        };
        if !epoch_side.contains(epoch, boundary_epoch) {
            continue;
        }

        match plan_writer_cancellation(&entry.status) {
            WriterCancelAction::Cancel => {
                if let Some(&escaped) = escaped_checkpoint_txs.get(&id) {
                    entry.commit_txid = L1TxId::from(escaped.commit_txid.0);
                    entry.reveal_txid = L1TxId::from(escaped.reveal_txid.0);
                    entry.payload_signature = None;
                    entry.status = L1BundleStatus::Retiring;
                    ctx.put_payload_entry(payload_idx, entry).with_context(|| {
                        format!("relink escaped L1 writer payload entry {payload_idx}")
                    })?;
                    stats.relinked_bundles += 1;
                    debug!(
                        payload_idx,
                        epoch,
                        commit_txid = %escaped.commit_txid,
                        reveal_txid = %escaped.reveal_txid,
                        "relinked checkpoint bundle to escaped broadcaster transactions"
                    );
                    continue;
                }

                entry.payload_signature = None;
                entry.status = L1BundleStatus::Abandoned;
                ctx.put_payload_entry(payload_idx, entry)
                    .with_context(|| format!("abandon L1 writer payload entry {payload_idx}"))?;
                stats.abandoned_bundles += 1;
            }
            WriterCancelAction::CancelAndInvalidate => {
                let (commit_status, reveal_status) = bundle_broadcast_statuses(ctx, &entry)?;
                if [commit_status.as_ref(), reveal_status.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(is_escaped_broadcast_status)
                {
                    if epoch_side.allows_relink() && entry.status != L1BundleStatus::Retiring {
                        entry.payload_signature = None;
                        entry.status = L1BundleStatus::Retiring;
                        ctx.put_payload_entry(payload_idx, entry).with_context(|| {
                            format!("retire escaped L1 writer payload entry {payload_idx}")
                        })?;
                    }
                    stats.left_published_bundles += 1;
                    debug!(
                        payload_idx,
                        epoch,
                        ?commit_status,
                        ?reveal_status,
                        "leaving checkpoint bundle with escaped broadcaster transaction"
                    );
                    continue;
                }

                stats.invalidated_txs += invalidate_unpublished_bundle_txs(ctx, &entry)?;
                if entry.status != L1BundleStatus::Abandoned {
                    entry.payload_signature = None;
                    entry.status = L1BundleStatus::Abandoned;
                    ctx.put_payload_entry(payload_idx, entry).with_context(|| {
                        format!("abandon unpublished L1 writer payload entry {payload_idx}")
                    })?;
                    stats.abandoned_bundles += 1;
                }
            }
            WriterCancelAction::Leave => {
                if epoch_side.allows_relink()
                    && entry.status != L1BundleStatus::Finalized
                    && entry.status != L1BundleStatus::Retiring
                {
                    entry.payload_signature = None;
                    entry.status = L1BundleStatus::Retiring;
                    ctx.put_payload_entry(payload_idx, entry).with_context(|| {
                        format!("retire escaped L1 writer payload entry {payload_idx}")
                    })?;
                    stats.left_published_bundles += 1;
                }
            }
        }
    }

    let next_intent_idx = ctx
        .next_intent_idx()
        .context("read next L1 writer intent index")?;
    let first_intent_idx = first_live_intent_idx(ctx, next_intent_idx, boundary_epoch, epoch_side)?;

    for intent_idx in first_intent_idx..next_intent_idx {
        let Some(mut intent) = ctx
            .intent_entry(intent_idx)
            .with_context(|| format!("read L1 writer intent entry {intent_idx}"))?
        else {
            continue;
        };
        let PayloadCheckpointRef::Checkpoint { epoch, .. } = inspect_l1_payload(intent.payload())
        else {
            continue;
        };
        if !epoch_side.contains(epoch, boundary_epoch) || intent.status == IntentStatus::Abandoned {
            continue;
        }

        let should_abandon = match intent.status {
            IntentStatus::Unbundled => true,
            IntentStatus::Bundled(payload_idx) => {
                match ctx.payload_entry(payload_idx).with_context(|| {
                    format!("read bundle {payload_idx} referenced by L1 writer intent {intent_idx}")
                })? {
                    // A finalized bundle in the unaccepted range is buried on L1 for good
                    // yet ASM never accepted its epoch, so nothing in the writer will
                    // replace it: the watcher stops tracking finalized bundles. Leaving the
                    // intent bundled would have the rebuilt checkpoint deduplicate against
                    // this dead submission and stall the epoch, because a deterministic
                    // rebuild reproduces the same intent commitment. Freeing the intent lets
                    // the retry allocate a fresh index and emit a replacement envelope.
                    Some(payload) => {
                        payload.status == L1BundleStatus::Abandoned
                            || (epoch_side.rebuilds_checkpoints()
                                && payload.status == L1BundleStatus::Finalized)
                    }
                    None => {
                        warn!(
                            intent_idx,
                            payload_idx,
                            epoch,
                            "checkpoint intent references a missing writer payload; abandoning intent"
                        );
                        stats.repaired_orphans += 1;
                        true
                    }
                }
            }
            IntentStatus::Abandoned => false,
        };

        if should_abandon {
            let intent_id = *intent.intent.commitment();
            intent.status = IntentStatus::Abandoned;
            ctx.update_intent_entry(intent_id, intent)
                .with_context(|| format!("abandon L1 writer intent entry {intent_idx}"))?;
            stats.abandoned_intents += 1;
        }
    }

    Ok(stats)
}

/// Sweeps the broadcaster queue for checkpoint transactions, invalidating the stale
/// ones and reporting the escaped ones.
///
/// Entries are found by decoding rather than by following bundle links: a crash can
/// leave a broadcaster entry that no bundle records, and those republish just the same.
fn invalidate_checkpoint_txs_by_decoding(
    ctx: &impl CheckpointCancelContext,
    boundary_epoch: Epoch,
    magic_bytes: [u8; 4],
    epoch_side: CheckpointEpochSide,
) -> Result<(usize, EscapedCheckpointTxs)> {
    let next_tx_idx = ctx
        .next_broadcast_idx()
        .context("read next L1 broadcaster transaction index")?;
    let magic = MagicBytes::new(magic_bytes);
    let mut invalidated = 0usize;
    let mut escaped_checkpoint_txs = EscapedCheckpointTxs::new();

    for tx_idx in 0..next_tx_idx {
        let Some(reveal_txid) = ctx
            .broadcast_txid(tx_idx)
            .with_context(|| format!("read L1 broadcaster transaction id {tx_idx}"))?
        else {
            continue;
        };
        let Some(mut tx_entry) = ctx
            .broadcast_entry(tx_idx)
            .with_context(|| format!("read L1 broadcaster transaction entry {tx_idx}"))?
        else {
            warn!(tx_idx, "L1 broadcaster transaction index has no entry");
            continue;
        };
        // Decoding is the expensive half of this sweep: a consensus decode of the raw
        // transaction followed by a taproot envelope parse, over payloads carrying a
        // proof and a state-diff sidecar. The broadcaster queue is never pruned, so
        // decode only what this pass could still act on. An escaped transaction is
        // exclusively recorded for relinking, which settled epochs never do, leaving
        // `Unpublished` as the only status worth looking inside there.
        //
        // `Unpublished` stays in scope at every index, however old: the broadcaster
        // reloads its queue from index 0 on startup and republishes anything still
        // unpublished, so an index bound here would put back the stale submissions this
        // pass exists to stop.
        let can_act_on_entry = tx_entry.status == L1TxStatus::Unpublished
            || (epoch_side.allows_relink() && is_escaped_broadcast_status(&tx_entry.status));
        if !can_act_on_entry {
            continue;
        }

        let tx: Transaction = match deserialize(tx_entry.tx_raw()) {
            Ok(tx) => tx,
            Err(err) => {
                warn!(
                    tx_idx,
                    %err,
                    "could not decode L1 broadcaster transaction during checkpoint reconciliation"
                );
                continue;
            }
        };
        let Some(checkpoint) = checkpoint_from_tx(&tx, magic) else {
            continue;
        };
        let epoch = checkpoint.epoch;
        if !epoch_side.contains(epoch, boundary_epoch) {
            continue;
        }

        let commit_txid = tx.input[0].previous_output.txid.to_buf32();
        if let Some(progress) = EscapeProgress::from_status(&tx_entry.status) {
            if epoch_side.allows_relink() {
                record_escaped_checkpoint_txs(
                    &mut escaped_checkpoint_txs,
                    checkpoint.id,
                    EscapedEnvelopeTxs {
                        commit_txid,
                        reveal_txid,
                        epoch,
                        progress,
                    },
                );
            }
            continue;
        }

        let commit_entry = ctx.broadcast_entry_by_id(commit_txid).with_context(|| {
            format!("read checkpoint commit broadcaster transaction {commit_txid}")
        })?;
        if commit_entry
            .as_ref()
            .is_some_and(|entry| is_escaped_broadcast_status(&entry.status))
        {
            // The commit already reached the network, so the envelope is in
            // flight even though the reveal has not been sent yet. Relink
            // unaccepted epochs; settled epochs only need the reveal left
            // publishable so the commit output is not stranded.
            if epoch_side.allows_relink() {
                record_escaped_checkpoint_txs(
                    &mut escaped_checkpoint_txs,
                    checkpoint.id,
                    EscapedEnvelopeTxs {
                        commit_txid,
                        reveal_txid,
                        epoch,
                        progress: EscapeProgress::CommitOnly,
                    },
                );
            }
            debug!(
                tx_idx,
                epoch,
                %commit_txid,
                "leaving queued checkpoint reveal whose commit transaction escaped"
            );
            continue;
        }

        if let Some(mut commit_entry) = commit_entry
            && commit_entry.status == L1TxStatus::Unpublished
        {
            commit_entry.status = L1TxStatus::InvalidInputs;
            ctx.put_broadcast_entry(commit_txid, commit_entry)
                .with_context(|| {
                    format!("invalidate checkpoint commit broadcaster transaction {commit_txid}")
                })?;
            invalidated += 1;
            // Leave the reveal publishable, exactly as the escaped-commit branch above
            // does, because `Unpublished` does not prove the commit stayed off the
            // network. If it escaped through the crash window, this reveal is the only
            // thing that can ever spend its output: the commit pays to a taproot key
            // that `EnvelopeSigningMode::InProcess` throws away after signing, so there
            // is no key-path spend, and invalidating both would leave the commit value
            // recoverable only by hand-rebroadcasting the reveal out of this database.
            // If the commit really is unsent, the reveal is an orphan, bitcoind rejects
            // it as `bad-txns-inputs-missingorspent`, and the broadcaster settles it at
            // `InvalidInputs` on its own — one wasted RPC, nothing on chain.
            debug!(
                tx_idx,
                epoch,
                %commit_txid,
                "invalidated queued checkpoint commit transaction, leaving its reveal publishable"
            );
            continue;
        }

        tx_entry.status = L1TxStatus::InvalidInputs;
        ctx.put_broadcast_entry_by_idx(tx_idx, tx_entry)
            .with_context(|| {
                format!("invalidate decoded checkpoint broadcaster transaction {tx_idx}")
            })?;
        invalidated += 1;
        debug!(
            tx_idx,
            epoch, "invalidated queued checkpoint broadcaster transaction"
        );
    }

    Ok((invalidated, escaped_checkpoint_txs))
}

/// Records an escaped commit/reveal pair for a checkpoint identity.
///
/// A collision here means the same checkpoint was broadcast more than once, which a
/// re-sign after [`L1BundleStatus::NeedsResign`] can do. Keep whichever envelope got
/// further, since relinking the bundle to the one closest to finalization is what lets
/// the watcher drive it to completion soonest.
fn record_escaped_checkpoint_txs(
    escaped_checkpoint_txs: &mut EscapedCheckpointTxs,
    checkpoint_id: Buf32,
    escaped: EscapedEnvelopeTxs,
) {
    match escaped_checkpoint_txs.get(&checkpoint_id) {
        Some(existing) if existing.progress >= escaped.progress => {
            warn!(
                epoch = escaped.epoch,
                %checkpoint_id,
                kept_reveal_txid = %existing.reveal_txid,
                kept_progress = ?existing.progress,
                dropped_reveal_txid = %escaped.reveal_txid,
                dropped_progress = ?escaped.progress,
                "multiple escaped envelopes for one checkpoint; keeping the furthest along"
            );
        }
        Some(existing) => {
            warn!(
                epoch = escaped.epoch,
                %checkpoint_id,
                kept_reveal_txid = %escaped.reveal_txid,
                kept_progress = ?escaped.progress,
                dropped_reveal_txid = %existing.reveal_txid,
                dropped_progress = ?existing.progress,
                "multiple escaped envelopes for one checkpoint; keeping the furthest along"
            );
            escaped_checkpoint_txs.insert(checkpoint_id, escaped);
        }
        None => {
            escaped_checkpoint_txs.insert(checkpoint_id, escaped);
        }
    }
}

/// Returns the broadcaster statuses of a bundle's commit and reveal transactions.
fn bundle_broadcast_statuses(
    ctx: &impl CheckpointCancelContext,
    entry: &BundledPayloadEntry,
) -> Result<(Option<L1TxStatus>, Option<L1TxStatus>)> {
    let commit_txid = Buf32::from(entry.commit_txid.0);
    let reveal_txid = Buf32::from(entry.reveal_txid.0);
    let commit_status = ctx
        .broadcast_entry_by_id(commit_txid)
        .with_context(|| format!("read broadcaster transaction {commit_txid}"))?
        .map(|entry| entry.status);
    let reveal_status = ctx
        .broadcast_entry_by_id(reveal_txid)
        .with_context(|| format!("read broadcaster transaction {reveal_txid}"))?
        .map(|entry| entry.status);
    Ok((commit_status, reveal_status))
}

/// Returns whether the broadcaster status proves the transaction reached the network.
///
/// `Unpublished` is not proof of the opposite: a crash between
/// `send_raw_transaction` and the broadcaster's status write leaves an
/// already-sent transaction reading `Unpublished`. Ruling that out would
/// require probing bitcoind during startup reconciliation; the
/// milliseconds-wide window is accepted instead. If such a reveal is
/// invalidated here and the original still gets mined, ASM accepts it and the
/// writer's epoch gate stops the rebuilt duplicate, or rejects it at a
/// bounded fee cost with no safety impact.
///
/// A commit read through that window is the costlier half, since a commit whose
/// reveal is gone strands its output rather than overpaying fees, so callers
/// leave the reveal publishable instead of invalidating the pair.
fn is_escaped_broadcast_status(status: &L1TxStatus) -> bool {
    matches!(
        status,
        L1TxStatus::Published | L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. }
    )
}

/// Returns the first bundle index that could still need cancelling.
///
/// Bundles finalize in index order, so nothing at or below the last finalized one is
/// still in flight.
fn first_payload_after_last_finalized(
    ctx: &impl CheckpointCancelContext,
    next_payload_idx: u64,
) -> Result<u64> {
    for payload_idx in (0..next_payload_idx).rev() {
        let Some(entry) = ctx
            .payload_entry(payload_idx)
            .with_context(|| format!("read L1 writer payload entry {payload_idx}"))?
        else {
            continue;
        };
        if entry.status == L1BundleStatus::Finalized {
            return Ok(payload_idx + 1);
        }
    }
    Ok(0)
}

/// Returns the first intent index this pass could still need to abandon.
///
/// The intent-side counterpart of [`first_payload_after_last_finalized`]: without a floor
/// every boot loads and SSZ-decodes the entire intent history, which no running node
/// prunes, so startup grows with sequencer lifetime.
///
/// The anchor differs per side because the two ranges sit at opposite ends of the index
/// space. Both walk down from the newest intent and stop at the first one that is out of
/// reach, which in a healthy queue is only a few entries in.
fn first_live_intent_idx(
    ctx: &impl CheckpointCancelContext,
    next_intent_idx: u64,
    boundary_epoch: Epoch,
    epoch_side: CheckpointEpochSide,
) -> Result<u64> {
    for intent_idx in (0..next_intent_idx).rev() {
        let Some(intent) = ctx
            .intent_entry(intent_idx)
            .with_context(|| format!("read L1 writer intent entry {intent_idx}"))?
        else {
            continue;
        };

        let out_of_reach = match epoch_side {
            // Settled epochs are the oldest ones, so the epoch cannot bound a scan that
            // starts at the newest intent. Anchor on finalization instead, exactly as the
            // bundler's own startup scan does: a settled pass never rebuilds a finalized
            // bundle, and the bundler stops at the same point, so an intent below it is
            // never picked up again either.
            //
            // Anchoring on the first merely *bundled* intent would be unsound, for the
            // reason [`IntentStatus::Abandoned`] documents: intent indices alias a
            // commitment-keyed entry, so a resubmitted intent can leave an unbundled alias
            // sitting below a bundled one.
            CheckpointEpochSide::Settled => match intent.status {
                IntentStatus::Bundled(payload_idx) => ctx
                    .payload_entry(payload_idx)
                    .with_context(|| {
                        format!(
                            "read bundle {payload_idx} referenced by L1 writer intent {intent_idx}"
                        )
                    })?
                    .is_some_and(|payload| payload.status == L1BundleStatus::Finalized),
                _ => false,
            },
            // Unaccepted epochs are the newest ones, so the epoch bounds the scan directly.
            // Checkpoint intents are appended in non-decreasing epoch order, so once the
            // walk drops below the boundary nothing under it is in range either. Finalized
            // bundles are no anchor on this side: freeing their intents is precisely what
            // the pass is here to do.
            CheckpointEpochSide::Unaccepted => {
                match inspect_l1_payload(intent.payload()) {
                    PayloadCheckpointRef::Checkpoint { epoch, .. } => {
                        !epoch_side.contains(epoch, boundary_epoch)
                    }
                    // Payloads of other subprotocols carry no epoch to compare.
                    _ => false,
                }
            }
        };

        if out_of_reach {
            return Ok(intent_idx + 1);
        }
    }
    Ok(0)
}

/// Invalidates a bundle's still-unpublished transactions.
///
/// The reveal only dies alongside a commit that was never queued for broadcast at all,
/// since that is the one case where no commit output can exist to strand. Whenever the
/// commit is in the broadcaster's hands the reveal stays publishable, for the reason
/// [`is_escaped_broadcast_status`] gives: `Unpublished` does not prove the commit stayed
/// off the network, and the reveal is the only thing that can spend its output.
///
/// This is the bundle-linked half of the rule that
/// [`invalidate_checkpoint_txs_by_decoding`] applies to unlinked entries. It has to
/// match, because the sweep runs first and this pass would otherwise invalidate the very
/// reveals the sweep deliberately left alone.
fn invalidate_unpublished_bundle_txs(
    ctx: &impl CheckpointCancelContext,
    entry: &BundledPayloadEntry,
) -> Result<usize> {
    let commit_txid = Buf32::from(entry.commit_txid.0);
    let commit_entry = ctx
        .broadcast_entry_by_id(commit_txid)
        .with_context(|| format!("read broadcaster transaction {commit_txid}"))?;
    let commit_is_queued = commit_entry.is_some();

    let mut invalidated = 0usize;
    if let Some(commit_entry) = commit_entry {
        invalidated += invalidate_unpublished_entry(ctx, commit_txid, commit_entry)?;
    }
    if commit_is_queued {
        return Ok(invalidated);
    }

    let reveal_txid = Buf32::from(entry.reveal_txid.0);
    let Some(reveal_entry) = ctx
        .broadcast_entry_by_id(reveal_txid)
        .with_context(|| format!("read broadcaster transaction {reveal_txid}"))?
    else {
        return Ok(invalidated);
    };
    invalidated += invalidate_unpublished_entry(ctx, reveal_txid, reveal_entry)?;
    Ok(invalidated)
}

/// Moves an `Unpublished` broadcaster entry to `InvalidInputs`, reporting whether it did.
fn invalidate_unpublished_entry(
    ctx: &impl CheckpointCancelContext,
    txid: Buf32,
    mut tx_entry: L1TxEntry,
) -> Result<usize> {
    if tx_entry.status != L1TxStatus::Unpublished {
        return Ok(0);
    }

    tx_entry.status = L1TxStatus::InvalidInputs;
    ctx.put_broadcast_entry(txid, tx_entry)
        .with_context(|| format!("invalidate broadcaster transaction {txid}"))?;
    Ok(1)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_asm_checkpoint_types::{
        CheckpointPayload, CheckpointSidecar, test_utils::create_test_checkpoint_payload,
    };
    use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
    use strata_asm_proto_txs_test_utils::{
        TEST_MAGIC_BYTES, create_dummy_tx, create_reveal_transaction_stub,
    };
    use strata_codec::encode_to_vec;
    use strata_codec_utils::CodecSsz;
    use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
    use strata_db_store_sled::{SledBackend, test_utils::get_test_sled_backend};
    use strata_db_types::{
        l1_broadcast::L1BroadcastDatabase,
        l1_writer::{IntentEntry, L1WriterDatabase},
    };
    use strata_storage::{create_node_storage, test_runtime_handle};

    use super::*;

    /// A checkpoint and the two encodings the queue holds it in.
    struct CheckpointFixture {
        /// Bytes the reveal transaction carries in its envelope.
        encoded: Vec<u8>,
        /// Payload the writer queue stores.
        l1_payload: L1Payload,
    }

    /// The transaction ids of a stored commit/reveal pair.
    #[derive(Clone, Copy)]
    struct EnvelopeTxids {
        commit: Buf32,
        reveal: Buf32,
    }

    /// Writer-queue indices of a queued checkpoint.
    struct Queued {
        intent_idx: u64,
        payload_idx: u64,
    }

    /// Node storage backed by a throwaway sled instance.
    struct Fixture {
        db: Arc<SledBackend>,
        storage: NodeStorage,
    }

    impl Fixture {
        fn new() -> Self {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("test: create node storage");
            Self { db, storage }
        }

        /// Builds the checkpoint for `epoch` without storing anything.
        fn checkpoint(&self, epoch: Epoch) -> CheckpointFixture {
            Self::encode_checkpoint(create_test_checkpoint_payload(epoch))
        }

        /// Builds a second, different checkpoint for `epoch`.
        ///
        /// A fork or a rebuild leaves several candidates for one epoch in flight, and
        /// they must not be treated as interchangeable.
        fn checkpoint_variant(&self, epoch: Epoch, variant: u8) -> CheckpointFixture {
            let base = create_test_checkpoint_payload(epoch);
            let sidecar = CheckpointSidecar::new(
                vec![variant; 100],
                base.sidecar().ol_logs().to_vec(),
                base.sidecar().terminal_header_complement().clone(),
            )
            .expect("test: build variant sidecar");
            let checkpoint = CheckpointPayload::new(*base.new_tip(), sidecar, vec![variant])
                .expect("test: build variant checkpoint");

            Self::encode_checkpoint(checkpoint)
        }

        /// Renders the two encodings the queue holds a checkpoint in.
        fn encode_checkpoint(checkpoint: CheckpointPayload) -> CheckpointFixture {
            let encoded =
                encode_to_vec(&CodecSsz::new(checkpoint)).expect("test: encode checkpoint payload");
            let l1_payload =
                L1Payload::new(vec![encoded.clone()], OL_STF_CHECKPOINT_TX_TAG.clone())
                    .expect("test: build L1 checkpoint payload");
            CheckpointFixture {
                encoded,
                l1_payload,
            }
        }

        /// Stores a tagged commit/reveal pair the broadcaster sweep can decode.
        ///
        /// `inputs` only varies the commit transaction so that several fixtures in one
        /// test do not collide on a transaction id.
        fn put_envelope_txs(
            &self,
            checkpoint: &CheckpointFixture,
            inputs: usize,
            commit_status: L1TxStatus,
            reveal_status: L1TxStatus,
        ) -> EnvelopeTxids {
            let commit_tx = create_dummy_tx(inputs, 1);
            let mut reveal_tx = create_reveal_transaction_stub(
                checkpoint.encoded.clone(),
                &OL_STF_CHECKPOINT_TX_TAG,
            );
            reveal_tx.input[0].previous_output.txid = commit_tx.compute_txid();

            EnvelopeTxids {
                commit: self.put_broadcast_tx(&commit_tx, commit_status),
                reveal: self.put_broadcast_tx(&reveal_tx, reveal_status),
            }
        }

        /// Stores broadcaster entries under forced ids, holding transactions the sweep
        /// cannot parse as checkpoints.
        fn put_opaque_txs(
            &self,
            seed: u8,
            commit_status: L1TxStatus,
            reveal_status: L1TxStatus,
        ) -> EnvelopeTxids {
            let tx = create_dummy_tx(1, 1);
            let ids = EnvelopeTxids {
                commit: Buf32::from([seed; 32]),
                reveal: Buf32::from([seed.wrapping_add(1); 32]),
            };
            for (txid, status) in [(ids.commit, commit_status), (ids.reveal, reveal_status)] {
                let mut entry = L1TxEntry::from_tx(&tx);
                entry.status = status;
                self.db
                    .broadcast_db()
                    .put_tx_entry(txid, entry)
                    .expect("test: store opaque broadcaster transaction");
            }
            ids
        }

        /// Stores an unparsable reveal entry whose bundle's commit id is never backed by
        /// an entry, modelling a bundle built before its commit reached the broadcaster.
        fn put_opaque_reveal_only(&self, seed: u8, reveal_status: L1TxStatus) -> EnvelopeTxids {
            let ids = EnvelopeTxids {
                commit: Buf32::from([seed; 32]),
                reveal: Buf32::from([seed.wrapping_add(1); 32]),
            };
            let mut entry = L1TxEntry::from_tx(&create_dummy_tx(1, 1));
            entry.status = reveal_status;
            self.db
                .broadcast_db()
                .put_tx_entry(ids.reveal, entry)
                .expect("test: store opaque broadcaster transaction");
            ids
        }

        /// Stores one broadcaster entry, returning its transaction id.
        fn put_broadcast_tx(&self, tx: &Transaction, status: L1TxStatus) -> Buf32 {
            let txid = tx.compute_txid().to_buf32();
            let mut entry = L1TxEntry::from_tx(tx);
            entry.status = status;
            self.db
                .broadcast_db()
                .put_tx_entry(txid, entry)
                .expect("test: store broadcaster transaction");
            txid
        }

        /// Queues an intent for `checkpoint` and bundles it at `status`.
        ///
        /// `txids` is what the bundle records; a bundle that never got as far as
        /// signing carries none, which is how an escaped envelope ends up orphaned.
        fn queue_bundle(
            &self,
            checkpoint: &CheckpointFixture,
            seed: u8,
            status: L1BundleStatus,
            txids: Option<EnvelopeTxids>,
        ) -> Queued {
            let (intent_id, intent_entry) = self.intent(checkpoint, seed);
            let writer_db = self.db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("test: store checkpoint intent");
            let bundle = match txids {
                Some(txids) => BundledPayloadEntry::new(
                    checkpoint.l1_payload.clone(),
                    L1TxId::from(txids.commit.0),
                    L1TxId::from(txids.reveal.0),
                    status,
                ),
                None => {
                    let mut bundle =
                        BundledPayloadEntry::new_unsigned(checkpoint.l1_payload.clone());
                    bundle.status = status;
                    bundle
                }
            };
            let payload_idx = writer_db
                .bundle_intent_payload(intent_id, intent_entry, bundle)
                .expect("test: bundle checkpoint intent");

            Queued {
                intent_idx,
                payload_idx,
            }
        }

        /// Queues an intent that points at a bundle which does not exist.
        fn queue_orphan_intent(
            &self,
            checkpoint: &CheckpointFixture,
            seed: u8,
            payload_idx: u64,
        ) -> u64 {
            let (intent_id, mut intent_entry) = self.intent(checkpoint, seed);
            intent_entry.status = IntentStatus::Bundled(payload_idx);
            self.db
                .writer_db()
                .put_intent_entry(intent_id, intent_entry)
                .expect("test: store orphaned intent")
        }

        fn intent(&self, checkpoint: &CheckpointFixture, seed: u8) -> (Buf32, IntentEntry) {
            let intent = PayloadIntent::new(
                PayloadDest::L1,
                Buf32::from([seed; 32]),
                checkpoint.l1_payload.clone(),
            );
            let intent_id = *intent.commitment();
            (intent_id, IntentEntry::new_unbundled(intent))
        }

        fn cancel(&self, first_unaccepted_epoch: Epoch) -> WriterCancelStats {
            cancel_queued_checkpoint_submissions(
                &self.storage,
                first_unaccepted_epoch.checked_sub(1),
                first_unaccepted_epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("test: cancel queued checkpoint submissions")
        }

        fn cancel_settled(&self, last_settled_epoch: Option<Epoch>) -> WriterCancelStats {
            cancel_settled_checkpoint_submissions(
                &self.storage,
                last_settled_epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("test: cancel settled checkpoint submissions")
        }

        fn bundle(&self, payload_idx: u64) -> BundledPayloadEntry {
            self.storage
                .l1_writer()
                .get_payload_entry_by_idx_blocking(payload_idx)
                .expect("test: read bundle")
                .expect("test: bundle exists")
        }

        fn intent_status(&self, intent_idx: u64) -> IntentStatus {
            self.storage
                .l1_writer()
                .get_intent_by_idx_blocking(intent_idx)
                .expect("test: read intent")
                .expect("test: intent exists")
                .status
        }

        fn tx_status(&self, txid: Buf32) -> L1TxStatus {
            self.db
                .broadcast_db()
                .get_tx_entry_by_id(txid)
                .expect("test: read broadcaster transaction")
                .expect("test: broadcaster transaction exists")
                .status
        }
    }

    #[test]
    fn cancellation_plan_matches_queue_decision_table() {
        let pending = L1BundleStatus::PendingRevealTxSign(Buf32::zero());
        for status in [
            L1BundleStatus::Unsigned,
            L1BundleStatus::NeedsResign,
            pending,
        ] {
            assert_eq!(
                plan_writer_cancellation(&status),
                WriterCancelAction::Cancel
            );
        }
        for status in [
            L1BundleStatus::Unpublished,
            L1BundleStatus::Abandoned,
            L1BundleStatus::Retiring,
        ] {
            assert_eq!(
                plan_writer_cancellation(&status),
                WriterCancelAction::CancelAndInvalidate
            );
        }
        for status in [
            L1BundleStatus::Published,
            L1BundleStatus::Confirmed,
            L1BundleStatus::Finalized,
        ] {
            assert_eq!(plan_writer_cancellation(&status), WriterCancelAction::Leave);
        }
    }

    #[test]
    fn settled_only_cancellation_leaves_unaccepted_queue_untouched() {
        let fixture = Fixture::new();
        let settled_epoch = 2;
        let first_unaccepted_epoch = settled_epoch + 1;

        let settled = fixture.checkpoint(settled_epoch);
        let settled_txids = fixture.put_envelope_txs(
            &settled,
            1,
            L1TxStatus::Unpublished,
            L1TxStatus::Unpublished,
        );
        let settled_queued = fixture.queue_bundle(
            &settled,
            18,
            L1BundleStatus::Unpublished,
            Some(settled_txids),
        );

        let unaccepted = fixture.checkpoint(first_unaccepted_epoch);
        let unaccepted_txids =
            fixture.put_envelope_txs(&unaccepted, 2, L1TxStatus::Published, L1TxStatus::Published);
        let unaccepted_queued =
            fixture.queue_bundle(&unaccepted, 19, L1BundleStatus::Unsigned, None);

        let stats = fixture.cancel_settled(Some(settled_epoch));

        assert_eq!(
            stats,
            WriterCancelStats {
                abandoned_intents: 1,
                abandoned_bundles: 1,
                invalidated_txs: 1,
                ..WriterCancelStats::default()
            }
        );
        assert_eq!(
            fixture.bundle(settled_queued.payload_idx).status,
            L1BundleStatus::Abandoned
        );
        assert_eq!(
            fixture.intent_status(settled_queued.intent_idx),
            IntentStatus::Abandoned
        );
        assert_eq!(
            fixture.tx_status(settled_txids.commit),
            L1TxStatus::InvalidInputs
        );
        // The reveal stays publishable so a commit that escaped through the crash window
        // is not stranded; an unsent commit makes it an orphan bitcoind refuses.
        assert_eq!(
            fixture.tx_status(settled_txids.reveal),
            L1TxStatus::Unpublished
        );

        let unaccepted_bundle = fixture.bundle(unaccepted_queued.payload_idx);
        assert_eq!(unaccepted_bundle.status, L1BundleStatus::Unsigned);
        assert_eq!(unaccepted_bundle.commit_txid, L1TxId::from([0; 32]));
        assert_eq!(unaccepted_bundle.reveal_txid, L1TxId::from([0; 32]));
        assert_eq!(
            fixture.intent_status(unaccepted_queued.intent_idx),
            IntentStatus::Bundled(unaccepted_queued.payload_idx)
        );
        for txid in [unaccepted_txids.commit, unaccepted_txids.reveal] {
            assert_eq!(fixture.tx_status(txid), L1TxStatus::Published);
        }
    }

    #[test]
    fn settled_cancellation_without_declared_finality_leaves_queue_untouched() {
        let fixture = Fixture::new();
        let checkpoint = fixture.checkpoint(0);
        let queued = fixture.queue_bundle(&checkpoint, 20, L1BundleStatus::Unsigned, None);

        let stats = fixture.cancel_settled(None);

        assert_eq!(stats, WriterCancelStats::default());
        assert_eq!(
            fixture.bundle(queued.payload_idx).status,
            L1BundleStatus::Unsigned
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Bundled(queued.payload_idx)
        );
    }

    /// The intent scan stops at the last finalized bundle instead of walking the whole
    /// never-pruned intent history on every boot. Nothing is lost by stopping there: the
    /// bundler's own startup scan uses the same anchor, so an intent below it is never
    /// picked up again either.
    #[test]
    fn cancellation_stops_scanning_intents_below_last_finalized_bundle() {
        let fixture = Fixture::new();
        let stale = fixture.checkpoint(4);
        let finalized_checkpoint = fixture.checkpoint(5);

        let (stale_id, stale_intent) = fixture.intent(&stale, 30);
        let stale_idx = fixture
            .db
            .writer_db()
            .put_intent_entry(stale_id, stale_intent)
            .expect("test: store stale unbundled intent");

        let finalized =
            fixture.queue_bundle(&finalized_checkpoint, 31, L1BundleStatus::Finalized, None);

        let stats = fixture.cancel_settled(Some(6));

        assert_eq!(stats, WriterCancelStats::default());
        assert_eq!(fixture.intent_status(stale_idx), IntentStatus::Unbundled);
        assert_eq!(
            fixture.intent_status(finalized.intent_idx),
            IntentStatus::Bundled(finalized.payload_idx)
        );
    }

    #[test]
    fn cancellation_does_not_relink_below_tip_unsigned_bundle() {
        let fixture = Fixture::new();
        let epoch = 3;
        let checkpoint = fixture.checkpoint(epoch);
        let txids =
            fixture.put_envelope_txs(&checkpoint, 1, L1TxStatus::Published, L1TxStatus::Published);
        let queued = fixture.queue_bundle(&checkpoint, 17, L1BundleStatus::Unsigned, None);

        let stats = fixture.cancel(epoch + 1);

        assert_eq!(
            stats,
            WriterCancelStats {
                abandoned_intents: 1,
                abandoned_bundles: 1,
                ..WriterCancelStats::default()
            }
        );
        assert_eq!(
            fixture.bundle(queued.payload_idx).status,
            L1BundleStatus::Abandoned
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Abandoned
        );
        for txid in [txids.commit, txids.reveal] {
            assert_eq!(fixture.tx_status(txid), L1TxStatus::Published);
        }
    }

    #[test]
    fn cancellation_leaves_below_tip_reveal_when_commit_escaped() {
        let fixture = Fixture::new();
        let epoch = 4;
        let checkpoint = fixture.checkpoint(epoch);
        let txids = fixture.put_envelope_txs(
            &checkpoint,
            1,
            L1TxStatus::Published,
            L1TxStatus::Unpublished,
        );

        let stats = fixture.cancel(epoch + 1);

        assert_eq!(stats, WriterCancelStats::default());
        assert_eq!(fixture.tx_status(txids.commit), L1TxStatus::Published);
        assert_eq!(fixture.tx_status(txids.reveal), L1TxStatus::Unpublished);
    }

    #[test]
    fn cancellation_retires_bundle_with_partial_escape() {
        let fixture = Fixture::new();
        let epoch = 4;
        let checkpoint = fixture.checkpoint(epoch);
        let txids = fixture.put_envelope_txs(
            &checkpoint,
            1,
            L1TxStatus::Published,
            L1TxStatus::Unpublished,
        );
        let queued =
            fixture.queue_bundle(&checkpoint, 13, L1BundleStatus::Unpublished, Some(txids));

        let stats = fixture.cancel(epoch);

        assert_eq!(
            stats,
            WriterCancelStats {
                left_published_bundles: 1,
                ..WriterCancelStats::default()
            }
        );
        assert_eq!(
            fixture.bundle(queued.payload_idx).status,
            L1BundleStatus::Retiring
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Bundled(queued.payload_idx)
        );
        assert_eq!(fixture.tx_status(txids.commit), L1TxStatus::Published);
        assert_eq!(fixture.tx_status(txids.reveal), L1TxStatus::Unpublished);
    }

    #[test]
    fn cancellation_relinks_published_orphan_checkpoint_txs() {
        let fixture = Fixture::new();
        let epoch = 5;
        let checkpoint = fixture.checkpoint(epoch);
        let txids =
            fixture.put_envelope_txs(&checkpoint, 1, L1TxStatus::Published, L1TxStatus::Published);
        let queued = fixture.queue_bundle(&checkpoint, 14, L1BundleStatus::Unsigned, None);

        let stats = fixture.cancel(epoch);

        assert_eq!(
            stats,
            WriterCancelStats {
                relinked_bundles: 1,
                ..WriterCancelStats::default()
            }
        );
        let bundle = fixture.bundle(queued.payload_idx);
        assert_eq!(bundle.status, L1BundleStatus::Retiring);
        assert_eq!(bundle.commit_txid, L1TxId::from(txids.commit.0));
        assert_eq!(bundle.reveal_txid, L1TxId::from(txids.reveal.0));
        assert!(bundle.payload_signature.is_none());
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Bundled(queued.payload_idx)
        );
        for txid in [txids.commit, txids.reveal] {
            assert_eq!(fixture.tx_status(txid), L1TxStatus::Published);
        }
    }

    #[test]
    fn cancellation_relinks_bundle_when_only_commit_escaped() {
        let fixture = Fixture::new();
        let epoch = 8;
        let checkpoint = fixture.checkpoint(epoch);
        let txids = fixture.put_envelope_txs(
            &checkpoint,
            1,
            L1TxStatus::Published,
            L1TxStatus::Unpublished,
        );
        let queued = fixture.queue_bundle(&checkpoint, 15, L1BundleStatus::Unsigned, None);

        let stats = fixture.cancel(epoch);

        assert_eq!(
            stats,
            WriterCancelStats {
                relinked_bundles: 1,
                ..WriterCancelStats::default()
            }
        );
        let bundle = fixture.bundle(queued.payload_idx);
        assert_eq!(bundle.status, L1BundleStatus::Retiring);
        assert_eq!(bundle.commit_txid, L1TxId::from(txids.commit.0));
        assert_eq!(bundle.reveal_txid, L1TxId::from(txids.reveal.0));
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Bundled(queued.payload_idx)
        );
        assert_eq!(fixture.tx_status(txids.reveal), L1TxStatus::Unpublished);
        assert_eq!(fixture.tx_status(txids.commit), L1TxStatus::Published);
    }

    #[test]
    fn cancellation_does_not_relink_escaped_checkpoint_from_other_epoch() {
        let fixture = Fixture::new();
        let escaped_epoch = 6;
        let bundle_epoch = 7;
        let escaped = fixture.checkpoint(escaped_epoch);
        let txids =
            fixture.put_envelope_txs(&escaped, 1, L1TxStatus::Published, L1TxStatus::Published);
        let bundled = fixture.checkpoint(bundle_epoch);
        let queued = fixture.queue_bundle(&bundled, 15, L1BundleStatus::Unsigned, None);

        let stats = fixture.cancel(escaped_epoch);

        assert_eq!(
            stats,
            WriterCancelStats {
                abandoned_intents: 1,
                abandoned_bundles: 1,
                ..WriterCancelStats::default()
            }
        );
        assert_eq!(
            fixture.bundle(queued.payload_idx).status,
            L1BundleStatus::Abandoned
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Abandoned
        );
        for txid in [txids.commit, txids.reveal] {
            assert_eq!(fixture.tx_status(txid), L1TxStatus::Published);
        }
    }

    /// Two candidates for one epoch are not interchangeable.
    ///
    /// Relinking the queued bundle to the other candidate's envelope would have the
    /// watcher finalize it against a checkpoint it does not hold: if ASM then rejects
    /// the escaped one, the epoch never settles while the bundle's intent stays
    /// `Bundled`, so nothing resubmits it.
    #[test]
    fn cancellation_does_not_relink_other_candidate_for_same_epoch() {
        let fixture = Fixture::new();
        let epoch = 6;
        let escaped = fixture.checkpoint(epoch);
        let txids =
            fixture.put_envelope_txs(&escaped, 1, L1TxStatus::Published, L1TxStatus::Published);
        let bundled = fixture.checkpoint_variant(epoch, 0xab);
        let queued = fixture.queue_bundle(&bundled, 20, L1BundleStatus::Unsigned, None);

        let stats = fixture.cancel(epoch);

        assert_eq!(
            stats,
            WriterCancelStats {
                abandoned_intents: 1,
                abandoned_bundles: 1,
                ..WriterCancelStats::default()
            }
        );
        let bundle = fixture.bundle(queued.payload_idx);
        assert_eq!(bundle.status, L1BundleStatus::Abandoned);
        assert_eq!(bundle.commit_txid, L1TxId::zero());
        assert_eq!(bundle.reveal_txid, L1TxId::zero());
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Abandoned
        );
        for txid in [txids.commit, txids.reveal] {
            assert_eq!(fixture.tx_status(txid), L1TxStatus::Published);
        }
    }

    /// The same checkpoint can escape twice, once per re-sign. Relinking to the one
    /// furthest along is what gets the bundle to finalization soonest.
    #[test]
    fn cancellation_relinks_to_furthest_along_duplicate_envelope() {
        let fixture = Fixture::new();
        let epoch = 9;
        let checkpoint = fixture.checkpoint(epoch);
        let published =
            fixture.put_envelope_txs(&checkpoint, 1, L1TxStatus::Published, L1TxStatus::Published);
        let confirmed = fixture.put_envelope_txs(
            &checkpoint,
            2,
            L1TxStatus::Published,
            L1TxStatus::Confirmed {
                confirmations: 3,
                block_hash: Buf32::zero(),
                block_height: 100,
            },
        );
        let queued = fixture.queue_bundle(&checkpoint, 21, L1BundleStatus::Unsigned, None);

        let stats = fixture.cancel(epoch);

        assert_eq!(
            stats,
            WriterCancelStats {
                relinked_bundles: 1,
                ..WriterCancelStats::default()
            }
        );
        let bundle = fixture.bundle(queued.payload_idx);
        assert_eq!(bundle.status, L1BundleStatus::Retiring);
        assert_eq!(bundle.commit_txid, L1TxId::from(confirmed.commit.0));
        assert_eq!(bundle.reveal_txid, L1TxId::from(confirmed.reveal.0));
        assert_eq!(fixture.tx_status(published.reveal), L1TxStatus::Published);
    }

    #[test]
    fn cancellation_repairs_checkpoint_intent_with_missing_bundle() {
        let fixture = Fixture::new();
        let checkpoint = fixture.checkpoint(3);
        let intent_idx = fixture.queue_orphan_intent(&checkpoint, 8, 99);

        let stats = fixture.cancel(3);

        assert_eq!(stats.repaired_orphans, 1);
        assert_eq!(stats.abandoned_intents, 1);
        assert_eq!(fixture.intent_status(intent_idx), IntentStatus::Abandoned);
    }

    #[test]
    fn abandoned_bundle_cancellation_is_idempotent() {
        let fixture = Fixture::new();
        let checkpoint = fixture.checkpoint(4);
        let txids = fixture.put_opaque_txs(3, L1TxStatus::Unpublished, L1TxStatus::Unpublished);
        let queued = fixture.queue_bundle(&checkpoint, 7, L1BundleStatus::Abandoned, Some(txids));

        let first = fixture.cancel(4);
        let second = fixture.cancel(4);

        assert_eq!(first.abandoned_intents, 1);
        assert_eq!(first.invalidated_txs, 1);
        assert_eq!(fixture.tx_status(txids.commit), L1TxStatus::InvalidInputs);
        assert_eq!(fixture.tx_status(txids.reveal), L1TxStatus::Unpublished);
        assert_eq!(second, WriterCancelStats::default());
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Abandoned
        );
    }

    #[test]
    fn published_unaccepted_checkpoint_bundle_is_retired() {
        let fixture = Fixture::new();
        let checkpoint = fixture.checkpoint(5);
        let txids = fixture.put_opaque_txs(5, L1TxStatus::Published, L1TxStatus::Published);
        let queued = fixture.queue_bundle(&checkpoint, 6, L1BundleStatus::Published, Some(txids));

        let stats = fixture.cancel(5);

        assert_eq!(
            stats,
            WriterCancelStats {
                left_published_bundles: 1,
                ..WriterCancelStats::default()
            }
        );
        assert_eq!(
            fixture.bundle(queued.payload_idx).status,
            L1BundleStatus::Retiring
        );
        assert!(matches!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Bundled(_)
        ));
    }

    #[test]
    fn cancellation_sweeps_unlinked_checkpoint_broadcaster_entry() {
        let fixture = Fixture::new();
        let epoch = 6;
        let checkpoint = fixture.checkpoint(epoch);
        let txids = fixture.put_envelope_txs(
            &checkpoint,
            1,
            L1TxStatus::Unpublished,
            L1TxStatus::Unpublished,
        );
        let non_checkpoint_txid =
            fixture.put_broadcast_tx(&create_dummy_tx(2, 1), L1TxStatus::Unpublished);

        let stats = fixture.cancel(epoch);

        assert_eq!(stats.invalidated_txs, 1);
        assert_eq!(fixture.tx_status(txids.commit), L1TxStatus::InvalidInputs);
        assert_eq!(fixture.tx_status(txids.reveal), L1TxStatus::Unpublished);
        assert_eq!(
            fixture.tx_status(non_checkpoint_txid),
            L1TxStatus::Unpublished
        );
    }

    /// The mirror of leaving a reveal publishable: with no commit entry there is no
    /// commit output that could be stranded, so nothing argues for keeping the reveal.
    #[test]
    fn cancellation_invalidates_bundle_reveal_when_commit_was_never_queued() {
        let fixture = Fixture::new();
        let epoch = 6;
        let checkpoint = fixture.checkpoint(epoch);
        let txids = fixture.put_opaque_reveal_only(9, L1TxStatus::Unpublished);
        let queued =
            fixture.queue_bundle(&checkpoint, 21, L1BundleStatus::Unpublished, Some(txids));

        let stats = fixture.cancel(epoch);

        assert_eq!(stats.invalidated_txs, 1);
        assert_eq!(fixture.tx_status(txids.reveal), L1TxStatus::InvalidInputs);
        assert_eq!(
            fixture.bundle(queued.payload_idx).status,
            L1BundleStatus::Abandoned
        );
    }
}
