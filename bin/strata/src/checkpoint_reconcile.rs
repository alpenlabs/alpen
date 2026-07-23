//! Reconciles local checkpoint artifacts against ASM-accepted state.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
#[cfg(feature = "sequencer")]
use bitcoin::{Transaction, consensus::deserialize};
use strata_asm_common::Subprotocol;
#[cfg(feature = "sequencer")]
use strata_asm_common::TxInputRef;
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
#[cfg(feature = "sequencer")]
use strata_asm_proto_checkpoint_txs::{
    CHECKPOINT_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE, extract_checkpoint_from_envelope,
};
#[cfg(feature = "sequencer")]
use strata_btc_types::TxidExt;
use strata_btcio::writer::checkpoint_payload_epoch;
use strata_checkpoint_types::CheckpointProofTask;
use strata_db_types::{
    backend::DatabaseBackend,
    common::L1TxId,
    l1_broadcast::{L1BroadcastDatabase, L1TxStatus},
    l1_writer::{BundledPayloadEntry, IntentStatus, L1BundleStatus},
};
use strata_identifiers::{Buf32, Epoch, EpochCommitment};
#[cfg(feature = "sequencer")]
use strata_l1_txfmt::{MagicBytes, ParseConfig};
use strata_node_context::NodeContext;
use strata_storage::NodeStorage;
use tracing::{debug, info, warn};

/// Reconciles checkpoint queue state against ASM's accepted checkpoint tip.
///
/// Every node cancels stale submissions for already-settled epochs. Nodes with
/// a local prover also cancel and rebuild submissions past the ASM verified tip
/// so a rotated OL image cannot reuse stale pre-rotation proof artifacts.
pub(crate) fn reconcile_unaccepted_checkpoint_artifacts(nodectx: &NodeContext) -> Result<()> {
    let Some(first_unaccepted_epoch) = first_unaccepted_checkpoint_epoch(nodectx)? else {
        return Ok(());
    };

    let storage = nodectx.storage();
    let magic_bytes = *nodectx.asm_params().magic.as_bytes();
    let prover_configured = nodectx.config().prover.is_some();
    let stats = if prover_configured {
        reconcile_unaccepted_checkpoint_artifacts_from_epoch(
            storage,
            first_unaccepted_epoch,
            magic_bytes,
        )?
    } else {
        ReconcileStats {
            writer: cancel_settled_checkpoint_submissions(
                storage,
                first_unaccepted_epoch,
                magic_bytes,
            )?,
            ..ReconcileStats::default()
        }
    };

    if stats.has_changes() {
        info!(
            first_unaccepted_epoch,
            prover_configured,
            abandoned_intents = stats.writer.abandoned_intents,
            abandoned_bundles = stats.writer.abandoned_bundles,
            left_published_bundles = stats.writer.left_published_bundles,
            relinked_bundles = stats.writer.relinked_bundles,
            invalidated_txs = stats.writer.invalidated_txs,
            repaired_orphans = stats.writer.repaired_orphans,
            deleted_payloads = stats.deleted_payloads,
            deleted_proofs = stats.deleted_proofs,
            deleted_tasks = stats.deleted_tasks,
            "reconciled checkpoint queue and local artifacts against ASM verified tip"
        );
    }

    Ok(())
}

fn reconcile_unaccepted_checkpoint_artifacts_from_epoch(
    storage: &NodeStorage,
    first_unaccepted_epoch: Epoch,
    magic_bytes: [u8; 4],
) -> Result<ReconcileStats> {
    let cancel_stats =
        cancel_queued_checkpoint_submissions(storage, first_unaccepted_epoch, magic_bytes)?;
    let mut cleanup_commitments =
        checkpoint_commitments_from_epoch(storage, first_unaccepted_epoch)?;

    let deleted_payloads = storage
        .ol_checkpoint()
        .del_local_checkpoint_payload_entries_from_epoch_blocking(first_unaccepted_epoch)
        .context("delete unaccepted local checkpoint payloads")?;
    extend_missing(&mut cleanup_commitments, deleted_payloads.iter().copied());

    let mut deleted_proofs = 0usize;
    let mut deleted_tasks = 0usize;

    for commitment in cleanup_commitments {
        if storage
            .checkpoint_proof()
            .del_proof(commitment)
            .with_context(|| format!("delete checkpoint proof for commitment {commitment}"))?
        {
            deleted_proofs += 1;
        }

        let task_key = CheckpointProofTask(commitment).to_key_bytes();
        if storage
            .prover_tasks()
            .delete_task(&task_key)
            .with_context(|| format!("delete checkpoint prover task for commitment {commitment}"))?
        {
            deleted_tasks += 1;
        }
    }

    Ok(ReconcileStats {
        writer: cancel_stats,
        deleted_payloads: deleted_payloads.len(),
        deleted_proofs,
        deleted_tasks,
    })
}

fn checkpoint_commitments_from_epoch(
    storage: &NodeStorage,
    first_unaccepted_epoch: Epoch,
) -> Result<Vec<EpochCommitment>> {
    let Some(last_summarized_epoch) = storage
        .ol_checkpoint()
        .get_last_summarized_epoch_blocking()
        .context("read last summarized checkpoint epoch")?
    else {
        return Ok(Vec::new());
    };

    if first_unaccepted_epoch > last_summarized_epoch {
        return Ok(Vec::new());
    }

    let mut commitments = Vec::new();
    for epoch in first_unaccepted_epoch..=last_summarized_epoch {
        let epoch_commitments = storage
            .ol_checkpoint()
            .get_epoch_commitments_at_blocking(epoch)
            .with_context(|| format!("read checkpoint commitments for epoch {epoch}"))?;
        extend_missing(&mut commitments, epoch_commitments);
    }

    Ok(commitments)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WriterCancelAction {
    Cancel,
    CancelAndInvalidate,
    Leave,
}

fn plan_writer_cancellation(status: &L1BundleStatus) -> WriterCancelAction {
    match status {
        L1BundleStatus::Unsigned
        | L1BundleStatus::PendingRevealTxSign(_)
        | L1BundleStatus::NeedsResign => WriterCancelAction::Cancel,
        L1BundleStatus::Unpublished | L1BundleStatus::Abandoned => {
            WriterCancelAction::CancelAndInvalidate
        }
        L1BundleStatus::Published | L1BundleStatus::Confirmed | L1BundleStatus::Finalized => {
            WriterCancelAction::Leave
        }
    }
}

type EscapedCheckpointTxs = BTreeMap<Epoch, (Buf32, Buf32)>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckpointEpochSide {
    Settled,
    Unaccepted,
}

impl CheckpointEpochSide {
    fn contains(self, epoch: Epoch, first_unaccepted_epoch: Epoch) -> bool {
        match self {
            Self::Settled => epoch < first_unaccepted_epoch,
            Self::Unaccepted => epoch >= first_unaccepted_epoch,
        }
    }

    #[cfg(feature = "sequencer")]
    fn allows_relink(self) -> bool {
        self == Self::Unaccepted
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct WriterCancelStats {
    abandoned_intents: usize,
    abandoned_bundles: usize,
    left_published_bundles: usize,
    relinked_bundles: usize,
    invalidated_txs: usize,
    repaired_orphans: usize,
}

impl WriterCancelStats {
    fn merge(&mut self, other: Self) {
        self.abandoned_intents += other.abandoned_intents;
        self.abandoned_bundles += other.abandoned_bundles;
        self.left_published_bundles += other.left_published_bundles;
        self.relinked_bundles += other.relinked_bundles;
        self.invalidated_txs += other.invalidated_txs;
        self.repaired_orphans += other.repaired_orphans;
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ReconcileStats {
    writer: WriterCancelStats,
    deleted_payloads: usize,
    deleted_proofs: usize,
    deleted_tasks: usize,
}

impl ReconcileStats {
    fn has_changes(self) -> bool {
        self != Self::default()
    }
}

fn cancel_queued_checkpoint_submissions(
    storage: &NodeStorage,
    first_unaccepted_epoch: Epoch,
    magic_bytes: [u8; 4],
) -> Result<WriterCancelStats> {
    let mut stats =
        cancel_settled_checkpoint_submissions(storage, first_unaccepted_epoch, magic_bytes)?;
    stats.merge(cancel_unaccepted_checkpoint_submissions(
        storage,
        first_unaccepted_epoch,
        magic_bytes,
    )?);
    Ok(stats)
}

fn cancel_settled_checkpoint_submissions(
    storage: &NodeStorage,
    first_unaccepted_epoch: Epoch,
    magic_bytes: [u8; 4],
) -> Result<WriterCancelStats> {
    cancel_checkpoint_submissions_for_epoch_side(
        storage,
        first_unaccepted_epoch,
        magic_bytes,
        CheckpointEpochSide::Settled,
    )
}

fn cancel_unaccepted_checkpoint_submissions(
    storage: &NodeStorage,
    first_unaccepted_epoch: Epoch,
    magic_bytes: [u8; 4],
) -> Result<WriterCancelStats> {
    cancel_checkpoint_submissions_for_epoch_side(
        storage,
        first_unaccepted_epoch,
        magic_bytes,
        CheckpointEpochSide::Unaccepted,
    )
}

fn cancel_checkpoint_submissions_for_epoch_side(
    storage: &NodeStorage,
    first_unaccepted_epoch: Epoch,
    magic_bytes: [u8; 4],
    epoch_side: CheckpointEpochSide,
) -> Result<WriterCancelStats> {
    let writer = storage.l1_writer();
    let mut stats = WriterCancelStats::default();
    let (invalidated_txs, escaped_checkpoint_txs) = invalidate_checkpoint_txs_by_decoding(
        storage,
        first_unaccepted_epoch,
        magic_bytes,
        epoch_side,
    )?;
    stats.invalidated_txs += invalidated_txs;
    let next_payload_idx = writer
        .get_next_payload_idx_blocking()
        .context("read next L1 writer payload index")?;
    let first_unfinalized_payload_idx =
        first_payload_after_last_finalized(storage, next_payload_idx)?;

    for payload_idx in first_unfinalized_payload_idx..next_payload_idx {
        let Some(mut entry) = writer
            .get_payload_entry_by_idx_blocking(payload_idx)
            .with_context(|| format!("read L1 writer payload entry {payload_idx}"))?
        else {
            continue;
        };
        let Some(epoch) = checkpoint_payload_epoch(&entry.payload) else {
            continue;
        };
        if !epoch_side.contains(epoch, first_unaccepted_epoch) {
            continue;
        }

        match plan_writer_cancellation(&entry.status) {
            WriterCancelAction::Cancel => {
                if let Some(&(commit_txid, reveal_txid)) = escaped_checkpoint_txs.get(&epoch) {
                    entry.commit_txid = L1TxId::from(commit_txid.0);
                    entry.reveal_txid = L1TxId::from(reveal_txid.0);
                    entry.payload_signature = None;
                    entry.status = L1BundleStatus::Unpublished;
                    writer
                        .put_payload_entry_blocking(payload_idx, entry)
                        .with_context(|| {
                            format!("relink escaped L1 writer payload entry {payload_idx}")
                        })?;
                    stats.relinked_bundles += 1;
                    debug!(
                        payload_idx,
                        epoch,
                        %commit_txid,
                        %reveal_txid,
                        "relinked checkpoint bundle to escaped broadcaster transactions"
                    );
                    continue;
                }

                entry.payload_signature = None;
                entry.status = L1BundleStatus::Abandoned;
                writer
                    .put_payload_entry_blocking(payload_idx, entry)
                    .with_context(|| format!("abandon L1 writer payload entry {payload_idx}"))?;
                stats.abandoned_bundles += 1;
            }
            WriterCancelAction::CancelAndInvalidate => {
                let (commit_status, reveal_status) = bundle_broadcast_statuses(storage, &entry)?;
                if [commit_status.as_ref(), reveal_status.as_ref()]
                    .into_iter()
                    .flatten()
                    .any(is_escaped_broadcast_status)
                {
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

                stats.invalidated_txs += invalidate_unpublished_bundle_txs(storage, &entry)?;
                if entry.status != L1BundleStatus::Abandoned {
                    entry.payload_signature = None;
                    entry.status = L1BundleStatus::Abandoned;
                    writer
                        .put_payload_entry_blocking(payload_idx, entry)
                        .with_context(|| {
                            format!("abandon unpublished L1 writer payload entry {payload_idx}")
                        })?;
                    stats.abandoned_bundles += 1;
                }
            }
            WriterCancelAction::Leave => {}
        }
    }

    let next_intent_idx = writer
        .get_next_intent_idx_blocking()
        .context("read next L1 writer intent index")?;
    for intent_idx in 0..next_intent_idx {
        let Some(mut intent) = writer
            .get_intent_by_idx_blocking(intent_idx)
            .with_context(|| format!("read L1 writer intent entry {intent_idx}"))?
        else {
            continue;
        };
        let Some(epoch) = checkpoint_payload_epoch(intent.payload()) else {
            continue;
        };
        if !epoch_side.contains(epoch, first_unaccepted_epoch)
            || intent.status == IntentStatus::Abandoned
        {
            continue;
        }

        let should_abandon = match intent.status {
            IntentStatus::Unbundled => true,
            IntentStatus::Bundled(payload_idx) => {
                match writer
                    .get_payload_entry_by_idx_blocking(payload_idx)
                    .with_context(|| {
                        format!(
                            "read bundle {payload_idx} referenced by L1 writer intent {intent_idx}"
                        )
                    })? {
                    Some(payload) => payload.status == L1BundleStatus::Abandoned,
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
            writer
                .update_intent_entry_blocking(intent_id, intent)
                .with_context(|| format!("abandon L1 writer intent entry {intent_idx}"))?;
            stats.abandoned_intents += 1;
        }
    }

    Ok(stats)
}

#[cfg(feature = "sequencer")]
fn invalidate_checkpoint_txs_by_decoding(
    storage: &NodeStorage,
    first_unaccepted_epoch: Epoch,
    magic_bytes: [u8; 4],
    epoch_side: CheckpointEpochSide,
) -> Result<(usize, EscapedCheckpointTxs)> {
    let broadcast_db = storage.db().broadcast_db();
    let next_tx_idx = broadcast_db
        .get_next_tx_idx()
        .context("read next L1 broadcaster transaction index")?;
    let parser = ParseConfig::new(MagicBytes::new(magic_bytes));
    let mut invalidated = 0usize;
    let mut escaped_checkpoint_txs = BTreeMap::new();

    for tx_idx in 0..next_tx_idx {
        let Some(reveal_txid) = broadcast_db
            .get_txid(tx_idx)
            .with_context(|| format!("read L1 broadcaster transaction id {tx_idx}"))?
        else {
            continue;
        };
        let Some(mut tx_entry) = broadcast_db
            .get_tx_entry(tx_idx)
            .with_context(|| format!("read L1 broadcaster transaction entry {tx_idx}"))?
        else {
            warn!(tx_idx, "L1 broadcaster transaction index has no entry");
            continue;
        };
        if tx_entry.status != L1TxStatus::Unpublished
            && !is_escaped_broadcast_status(&tx_entry.status)
        {
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
        let Ok(tag) = parser.try_parse_tx(&tx) else {
            continue;
        };
        if tag.subproto_id() != CHECKPOINT_SUBPROTOCOL_ID
            || tag.tx_type() != OL_STF_CHECKPOINT_TX_TYPE
        {
            continue;
        }

        let checkpoint = match extract_checkpoint_from_envelope(&TxInputRef::new(&tx, tag)) {
            Ok(checkpoint) => checkpoint,
            Err(err) => {
                warn!(
                    tx_idx,
                    %err,
                    "could not extract checkpoint from tagged L1 broadcaster transaction"
                );
                continue;
            }
        };
        let epoch = checkpoint.payload.new_tip().epoch;
        if !epoch_side.contains(epoch, first_unaccepted_epoch) {
            continue;
        }

        let commit_txid = tx.input[0].previous_output.txid.to_buf32();
        if is_escaped_broadcast_status(&tx_entry.status) {
            if epoch_side.allows_relink() {
                record_escaped_checkpoint_txs(
                    &mut escaped_checkpoint_txs,
                    epoch,
                    commit_txid,
                    reveal_txid,
                );
            }
            continue;
        }

        let commit_entry = broadcast_db
            .get_tx_entry_by_id(commit_txid)
            .with_context(|| {
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
                    epoch,
                    commit_txid,
                    reveal_txid,
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
            broadcast_db
                .put_tx_entry(commit_txid, commit_entry)
                .with_context(|| {
                    format!("invalidate checkpoint commit broadcaster transaction {commit_txid}")
                })?;
            invalidated += 1;
            debug!(
                tx_idx,
                epoch,
                %commit_txid,
                "invalidated queued checkpoint commit transaction"
            );
        }

        tx_entry.status = L1TxStatus::InvalidInputs;
        broadcast_db
            .put_tx_entry_by_idx(tx_idx, tx_entry)
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

#[cfg(not(feature = "sequencer"))]
fn invalidate_checkpoint_txs_by_decoding(
    _storage: &NodeStorage,
    _first_unaccepted_epoch: Epoch,
    _magic_bytes: [u8; 4],
    _epoch_side: CheckpointEpochSide,
) -> Result<(usize, EscapedCheckpointTxs)> {
    Ok((0, BTreeMap::new()))
}

/// Records an escaped commit/reveal pair for an epoch, keeping the first pair
/// found and warning on duplicates.
#[cfg(feature = "sequencer")]
fn record_escaped_checkpoint_txs(
    escaped_checkpoint_txs: &mut EscapedCheckpointTxs,
    epoch: Epoch,
    commit_txid: Buf32,
    reveal_txid: Buf32,
) {
    if let Some(&(first_commit_txid, first_reveal_txid)) = escaped_checkpoint_txs.get(&epoch) {
        warn!(
            epoch,
            %first_commit_txid,
            %first_reveal_txid,
            %commit_txid,
            %reveal_txid,
            "multiple escaped checkpoint transactions found for epoch; keeping first"
        );
    } else {
        escaped_checkpoint_txs.insert(epoch, (commit_txid, reveal_txid));
    }
}

fn bundle_broadcast_statuses(
    storage: &NodeStorage,
    entry: &BundledPayloadEntry,
) -> Result<(Option<L1TxStatus>, Option<L1TxStatus>)> {
    let broadcast_db = storage.db().broadcast_db();
    let commit_txid = Buf32::from(entry.commit_txid.0);
    let reveal_txid = Buf32::from(entry.reveal_txid.0);
    let commit_status = broadcast_db
        .get_tx_entry_by_id(commit_txid)
        .with_context(|| format!("read broadcaster transaction {commit_txid}"))?
        .map(|entry| entry.status);
    let reveal_status = broadcast_db
        .get_tx_entry_by_id(reveal_txid)
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
/// milliseconds-wide window is accepted instead. If such a transaction is
/// invalidated here and the original still gets mined, ASM accepts it and the
/// writer's epoch gate stops the rebuilt duplicate, or rejects it at a
/// bounded fee cost with no safety impact.
fn is_escaped_broadcast_status(status: &L1TxStatus) -> bool {
    matches!(
        status,
        L1TxStatus::Published | L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. }
    )
}

fn first_payload_after_last_finalized(storage: &NodeStorage, next_payload_idx: u64) -> Result<u64> {
    let writer = storage.l1_writer();
    for payload_idx in (0..next_payload_idx).rev() {
        let Some(entry) = writer
            .get_payload_entry_by_idx_blocking(payload_idx)
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

fn invalidate_unpublished_bundle_txs(
    storage: &NodeStorage,
    entry: &BundledPayloadEntry,
) -> Result<usize> {
    let broadcast_db = storage.db().broadcast_db();
    let mut invalidated = 0usize;
    for txid in [entry.commit_txid, entry.reveal_txid] {
        let txid = Buf32::from(txid.0);
        let Some(mut tx_entry) = broadcast_db
            .get_tx_entry_by_id(txid)
            .with_context(|| format!("read broadcaster transaction {txid}"))?
        else {
            continue;
        };
        if tx_entry.status != L1TxStatus::Unpublished {
            continue;
        }

        tx_entry.status = L1TxStatus::InvalidInputs;
        broadcast_db
            .put_tx_entry(txid, tx_entry)
            .with_context(|| format!("invalidate broadcaster transaction {txid}"))?;
        invalidated += 1;
    }
    Ok(invalidated)
}

fn extend_missing<T>(items: &mut Vec<T>, candidates: impl IntoIterator<Item = T>)
where
    T: Copy + Eq,
{
    for candidate in candidates {
        if !items.contains(&candidate) {
            items.push(candidate);
        }
    }
}

fn first_unaccepted_checkpoint_epoch(nodectx: &NodeContext) -> Result<Option<Epoch>> {
    let Some((asm_l1, asm_state)) = nodectx
        .storage()
        .fetch_canonical_asm_state_blocking()
        .context("fetch canonical ASM state")?
    else {
        debug!("canonical ASM state is not available; skipping checkpoint artifact reconciliation");
        return Ok(None);
    };

    let checkpoint_state = asm_state
        .state()
        .find_section(<CheckpointSubprotocol as Subprotocol>::ID)
        .context("latest ASM state is missing checkpoint subprotocol state")?
        .try_to_state::<CheckpointSubprotocol>()
        .context("decode checkpoint subprotocol state")?;

    let verified_epoch = checkpoint_state.verified_tip().epoch;
    let Some(first_unaccepted_epoch) = verified_epoch.checked_add(1) else {
        debug!(
            %asm_l1,
            verified_epoch,
            "ASM checkpoint verified tip is at maximum epoch; no checkpoint artifacts to reconcile"
        );
        return Ok(None);
    };

    debug!(
        %asm_l1,
        verified_epoch,
        first_unaccepted_epoch,
        "resolved first unaccepted checkpoint epoch from ASM verified tip"
    );

    Ok(Some(first_unaccepted_epoch))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for status in [L1BundleStatus::Unpublished, L1BundleStatus::Abandoned] {
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

    #[cfg(feature = "sequencer")]
    mod sequencer {
        use bitcoin::{Transaction, absolute::LockTime, transaction::Version};
        use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
        use strata_asm_proto_checkpoint_types::{
            CheckpointPayload, test_utils::create_test_checkpoint_payload,
        };
        use strata_asm_proto_txs_test_utils::{
            TEST_MAGIC_BYTES, create_dummy_tx, create_reveal_transaction_stub,
        };
        use strata_codec::encode_to_vec;
        use strata_codec_utils::CodecSsz;
        use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
        use strata_db_store_sled::test_utils::get_test_sled_backend;
        use strata_db_types::{
            l1_broadcast::{L1TxEntry, L1TxStatus},
            l1_writer::{
                BundledPayloadEntry, IntentEntry, IntentStatus, L1BundleStatus, L1WriterDatabase,
            },
        };
        use strata_storage::{create_node_storage, test_runtime_handle};

        use super::super::*;

        fn checkpoint_commitment(payload: &CheckpointPayload) -> EpochCommitment {
            EpochCommitment::from_terminal(
                payload.new_tip().epoch,
                *payload.new_tip().l2_commitment(),
            )
        }

        fn checkpoint_l1_payload(epoch: Epoch) -> (CheckpointPayload, L1Payload) {
            let checkpoint = create_test_checkpoint_payload(epoch);
            let encoded = encode_to_vec(&CodecSsz::new(checkpoint.clone()))
                .expect("encode checkpoint payload");
            let payload = L1Payload::new(vec![encoded], OL_STF_CHECKPOINT_TX_TAG.clone())
                .expect("build L1 checkpoint payload");
            (checkpoint, payload)
        }

        fn test_transaction() -> Transaction {
            Transaction {
                version: Version(2),
                lock_time: LockTime::ZERO,
                input: Vec::new(),
                output: Vec::new(),
            }
        }

        #[test]
        fn reconcile_deletes_artifacts_and_cancels_unpublished_queue_entry() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let epoch = 2;
            let (checkpoint, l1_payload) = checkpoint_l1_payload(epoch);
            let commitment = checkpoint_commitment(&checkpoint);
            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([9; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let writer_db = db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store checkpoint intent");

            let commit_txid = L1TxId::from([1; 32]);
            let reveal_txid = L1TxId::from([2; 32]);
            let bundle = BundledPayloadEntry::new(
                l1_payload,
                commit_txid,
                reveal_txid,
                L1BundleStatus::Unpublished,
            );
            let payload_idx = writer_db
                .bundle_intent_payload(intent_id, intent_entry, bundle)
                .expect("bundle checkpoint intent");

            let broadcast_db = db.broadcast_db();
            for txid in [commit_txid, reveal_txid] {
                broadcast_db
                    .put_tx_entry(Buf32::from(txid.0), L1TxEntry::from_tx(&test_transaction()))
                    .expect("store unpublished transaction");
            }
            storage
                .ol_checkpoint()
                .put_checkpoint_payload_entry_blocking(commitment, checkpoint)
                .expect("store local checkpoint payload");
            storage
                .ol_checkpoint()
                .put_checkpoint_signing_entry_blocking(commitment, intent_idx)
                .expect("store checkpoint signing marker");

            let stats = reconcile_unaccepted_checkpoint_artifacts_from_epoch(
                &storage,
                epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("reconcile unaccepted checkpoint");

            assert_eq!(stats.deleted_payloads, 1);
            assert_eq!(stats.writer.abandoned_bundles, 1);
            assert_eq!(stats.writer.abandoned_intents, 1);
            assert_eq!(stats.writer.invalidated_txs, 2);
            assert!(
                storage
                    .ol_checkpoint()
                    .get_checkpoint_payload_entry_blocking(commitment)
                    .expect("read checkpoint payload")
                    .is_none()
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_payload_entry_by_idx_blocking(payload_idx)
                    .expect("read bundle")
                    .expect("bundle exists")
                    .status,
                L1BundleStatus::Abandoned
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read intent")
                    .expect("intent exists")
                    .status,
                IntentStatus::Abandoned
            );
            for txid in [commit_txid, reveal_txid] {
                assert_eq!(
                    broadcast_db
                        .get_tx_entry_by_id(Buf32::from(txid.0))
                        .expect("read broadcast transaction")
                        .expect("broadcast transaction exists")
                        .status,
                    L1TxStatus::InvalidInputs
                );
            }
        }

        #[test]
        fn reconcile_cancels_below_tip_bundle_without_deleting_settled_artifacts() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let epoch = 2;
            let first_unaccepted_epoch = epoch + 1;
            let (checkpoint, l1_payload) = checkpoint_l1_payload(epoch);
            let commitment = checkpoint_commitment(&checkpoint);
            let encoded = encode_to_vec(&CodecSsz::new(checkpoint.clone()))
                .expect("encode settled checkpoint payload");
            let commit_tx = create_dummy_tx(1, 1);
            let mut reveal_tx = create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
            reveal_tx.input[0].previous_output.txid = commit_tx.compute_txid();
            let commit_txid = commit_tx.compute_txid().to_buf32();
            let reveal_txid = reveal_tx.compute_txid().to_buf32();
            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([16; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let writer_db = db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store settled checkpoint intent");

            let payload_idx = writer_db
                .bundle_intent_payload(
                    intent_id,
                    intent_entry,
                    BundledPayloadEntry::new(
                        l1_payload,
                        L1TxId::from(commit_txid.0),
                        L1TxId::from(reveal_txid.0),
                        L1BundleStatus::Unpublished,
                    ),
                )
                .expect("bundle settled checkpoint intent");

            let broadcast_db = db.broadcast_db();
            for (txid, tx) in [(commit_txid, &commit_tx), (reveal_txid, &reveal_tx)] {
                broadcast_db
                    .put_tx_entry(txid, L1TxEntry::from_tx(tx))
                    .expect("store settled unpublished transaction");
            }
            storage
                .ol_checkpoint()
                .put_checkpoint_payload_entry_blocking(commitment, checkpoint)
                .expect("store settled checkpoint payload");

            let stats = reconcile_unaccepted_checkpoint_artifacts_from_epoch(
                &storage,
                first_unaccepted_epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("reconcile settled checkpoint queue entry");

            assert_eq!(
                stats,
                ReconcileStats {
                    writer: WriterCancelStats {
                        abandoned_intents: 1,
                        abandoned_bundles: 1,
                        invalidated_txs: 2,
                        ..WriterCancelStats::default()
                    },
                    ..ReconcileStats::default()
                }
            );
            assert!(
                storage
                    .ol_checkpoint()
                    .get_checkpoint_payload_entry_blocking(commitment)
                    .expect("read settled checkpoint payload")
                    .is_some()
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_payload_entry_by_idx_blocking(payload_idx)
                    .expect("read settled bundle")
                    .expect("settled bundle exists")
                    .status,
                L1BundleStatus::Abandoned
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read settled intent")
                    .expect("settled intent exists")
                    .status,
                IntentStatus::Abandoned
            );
            for txid in [commit_txid, reveal_txid] {
                assert_eq!(
                    broadcast_db
                        .get_tx_entry_by_id(txid)
                        .expect("read settled broadcaster transaction")
                        .expect("settled broadcaster transaction exists")
                        .status,
                    L1TxStatus::InvalidInputs
                );
            }
        }

        #[test]
        fn settled_only_reconcile_leaves_unaccepted_queue_and_artifacts_untouched() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let settled_epoch = 2;
            let first_unaccepted_epoch = settled_epoch + 1;
            let (settled_checkpoint, settled_payload) = checkpoint_l1_payload(settled_epoch);
            let settled_encoded = encode_to_vec(&CodecSsz::new(settled_checkpoint))
                .expect("encode settled checkpoint payload");
            let settled_commit_tx = create_dummy_tx(1, 1);
            let mut settled_reveal_tx =
                create_reveal_transaction_stub(settled_encoded, &OL_STF_CHECKPOINT_TX_TAG);
            settled_reveal_tx.input[0].previous_output.txid = settled_commit_tx.compute_txid();
            let settled_commit_txid = settled_commit_tx.compute_txid().to_buf32();
            let settled_reveal_txid = settled_reveal_tx.compute_txid().to_buf32();

            let writer_db = db.writer_db();
            let settled_intent = PayloadIntent::new(
                PayloadDest::L1,
                Buf32::from([18; 32]),
                settled_payload.clone(),
            );
            let settled_intent_id = *settled_intent.commitment();
            let settled_intent_entry = IntentEntry::new_unbundled(settled_intent);
            let settled_intent_idx = writer_db
                .put_intent_entry(settled_intent_id, settled_intent_entry.clone())
                .expect("store settled checkpoint intent");
            let settled_payload_idx = writer_db
                .bundle_intent_payload(
                    settled_intent_id,
                    settled_intent_entry,
                    BundledPayloadEntry::new(
                        settled_payload,
                        L1TxId::from(settled_commit_txid.0),
                        L1TxId::from(settled_reveal_txid.0),
                        L1BundleStatus::Unpublished,
                    ),
                )
                .expect("bundle settled checkpoint intent");

            let (unaccepted_checkpoint, unaccepted_payload) =
                checkpoint_l1_payload(first_unaccepted_epoch);
            let unaccepted_commitment = checkpoint_commitment(&unaccepted_checkpoint);
            let unaccepted_encoded = encode_to_vec(&CodecSsz::new(unaccepted_checkpoint.clone()))
                .expect("encode unaccepted checkpoint payload");
            let unaccepted_commit_tx = create_dummy_tx(2, 1);
            let mut unaccepted_reveal_tx =
                create_reveal_transaction_stub(unaccepted_encoded, &OL_STF_CHECKPOINT_TX_TAG);
            unaccepted_reveal_tx.input[0].previous_output.txid =
                unaccepted_commit_tx.compute_txid();
            let unaccepted_commit_txid = unaccepted_commit_tx.compute_txid().to_buf32();
            let unaccepted_reveal_txid = unaccepted_reveal_tx.compute_txid().to_buf32();

            let unaccepted_intent = PayloadIntent::new(
                PayloadDest::L1,
                Buf32::from([19; 32]),
                unaccepted_payload.clone(),
            );
            let unaccepted_intent_id = *unaccepted_intent.commitment();
            let unaccepted_intent_entry = IntentEntry::new_unbundled(unaccepted_intent);
            let unaccepted_intent_idx = writer_db
                .put_intent_entry(unaccepted_intent_id, unaccepted_intent_entry.clone())
                .expect("store unaccepted checkpoint intent");
            let unaccepted_payload_idx = writer_db
                .bundle_intent_payload(
                    unaccepted_intent_id,
                    unaccepted_intent_entry,
                    BundledPayloadEntry::new_unsigned(unaccepted_payload),
                )
                .expect("bundle unaccepted checkpoint intent");

            let broadcast_db = db.broadcast_db();
            for (txid, tx) in [
                (settled_commit_txid, &settled_commit_tx),
                (settled_reveal_txid, &settled_reveal_tx),
            ] {
                broadcast_db
                    .put_tx_entry(txid, L1TxEntry::from_tx(tx))
                    .expect("store settled unpublished transaction");
            }
            for (txid, tx) in [
                (unaccepted_commit_txid, &unaccepted_commit_tx),
                (unaccepted_reveal_txid, &unaccepted_reveal_tx),
            ] {
                let mut tx_entry = L1TxEntry::from_tx(tx);
                tx_entry.status = L1TxStatus::Published;
                broadcast_db
                    .put_tx_entry(txid, tx_entry)
                    .expect("store escaped unaccepted transaction");
            }
            storage
                .ol_checkpoint()
                .put_checkpoint_payload_entry_blocking(unaccepted_commitment, unaccepted_checkpoint)
                .expect("store unaccepted checkpoint payload");
            storage
                .ol_checkpoint()
                .put_checkpoint_signing_entry_blocking(unaccepted_commitment, unaccepted_intent_idx)
                .expect("store unaccepted checkpoint signing marker");

            let stats = cancel_settled_checkpoint_submissions(
                &storage,
                first_unaccepted_epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("reconcile settled checkpoint submissions");

            assert_eq!(
                stats,
                WriterCancelStats {
                    abandoned_intents: 1,
                    abandoned_bundles: 1,
                    invalidated_txs: 2,
                    ..WriterCancelStats::default()
                }
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_payload_entry_by_idx_blocking(settled_payload_idx)
                    .expect("read settled bundle")
                    .expect("settled bundle exists")
                    .status,
                L1BundleStatus::Abandoned
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(settled_intent_idx)
                    .expect("read settled intent")
                    .expect("settled intent exists")
                    .status,
                IntentStatus::Abandoned
            );
            for txid in [settled_commit_txid, settled_reveal_txid] {
                assert_eq!(
                    broadcast_db
                        .get_tx_entry_by_id(txid)
                        .expect("read settled transaction")
                        .expect("settled transaction exists")
                        .status,
                    L1TxStatus::InvalidInputs
                );
            }

            let unaccepted_bundle = storage
                .l1_writer()
                .get_payload_entry_by_idx_blocking(unaccepted_payload_idx)
                .expect("read unaccepted bundle")
                .expect("unaccepted bundle exists");
            assert_eq!(unaccepted_bundle.status, L1BundleStatus::Unsigned);
            assert_eq!(unaccepted_bundle.commit_txid, L1TxId::from([0; 32]));
            assert_eq!(unaccepted_bundle.reveal_txid, L1TxId::from([0; 32]));
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(unaccepted_intent_idx)
                    .expect("read unaccepted intent")
                    .expect("unaccepted intent exists")
                    .status,
                IntentStatus::Bundled(unaccepted_payload_idx)
            );
            assert!(
                storage
                    .ol_checkpoint()
                    .get_checkpoint_payload_entry_blocking(unaccepted_commitment)
                    .expect("read unaccepted checkpoint payload")
                    .is_some()
            );
            assert_eq!(
                storage
                    .ol_checkpoint()
                    .get_checkpoint_signing_entry_blocking(unaccepted_commitment)
                    .expect("read unaccepted checkpoint signing marker"),
                Some(unaccepted_intent_idx)
            );
            for txid in [unaccepted_commit_txid, unaccepted_reveal_txid] {
                assert_eq!(
                    broadcast_db
                        .get_tx_entry_by_id(txid)
                        .expect("read escaped unaccepted transaction")
                        .expect("escaped unaccepted transaction exists")
                        .status,
                    L1TxStatus::Published
                );
            }
        }

        #[test]
        fn cancellation_does_not_relink_below_tip_unsigned_bundle() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let epoch = 3;
            let first_unaccepted_epoch = epoch + 1;
            let (checkpoint, l1_payload) = checkpoint_l1_payload(epoch);
            let encoded =
                encode_to_vec(&CodecSsz::new(checkpoint)).expect("encode checkpoint payload");
            let commit_tx = create_dummy_tx(1, 1);
            let mut reveal_tx = create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
            reveal_tx.input[0].previous_output.txid = commit_tx.compute_txid();
            let commit_txid = commit_tx.compute_txid().to_buf32();
            let reveal_txid = reveal_tx.compute_txid().to_buf32();

            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([17; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let writer_db = db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store settled checkpoint intent");
            let payload_idx = writer_db
                .bundle_intent_payload(
                    intent_id,
                    intent_entry,
                    BundledPayloadEntry::new_unsigned(l1_payload),
                )
                .expect("bundle settled checkpoint intent");

            let broadcast_db = db.broadcast_db();
            for (txid, tx) in [(commit_txid, &commit_tx), (reveal_txid, &reveal_tx)] {
                let mut tx_entry = L1TxEntry::from_tx(tx);
                tx_entry.status = L1TxStatus::Published;
                broadcast_db
                    .put_tx_entry(txid, tx_entry)
                    .expect("store escaped settled transaction");
            }

            let stats = cancel_queued_checkpoint_submissions(
                &storage,
                first_unaccepted_epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("cancel settled unsigned checkpoint bundle");

            assert_eq!(
                stats,
                WriterCancelStats {
                    abandoned_intents: 1,
                    abandoned_bundles: 1,
                    ..WriterCancelStats::default()
                }
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_payload_entry_by_idx_blocking(payload_idx)
                    .expect("read settled bundle")
                    .expect("settled bundle exists")
                    .status,
                L1BundleStatus::Abandoned
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read settled intent")
                    .expect("settled intent exists")
                    .status,
                IntentStatus::Abandoned
            );
            for txid in [commit_txid, reveal_txid] {
                assert_eq!(
                    broadcast_db
                        .get_tx_entry_by_id(txid)
                        .expect("read escaped settled transaction")
                        .expect("escaped settled transaction exists")
                        .status,
                    L1TxStatus::Published
                );
            }
        }

        #[test]
        fn cancellation_leaves_below_tip_reveal_when_commit_escaped() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let epoch = 4;
            let first_unaccepted_epoch = epoch + 1;
            let (checkpoint, _) = checkpoint_l1_payload(epoch);
            let encoded =
                encode_to_vec(&CodecSsz::new(checkpoint)).expect("encode checkpoint payload");
            let commit_tx = create_dummy_tx(1, 1);
            let mut reveal_tx = create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
            reveal_tx.input[0].previous_output.txid = commit_tx.compute_txid();
            let commit_txid = commit_tx.compute_txid().to_buf32();
            let reveal_txid = reveal_tx.compute_txid().to_buf32();

            let broadcast_db = db.broadcast_db();
            let mut commit_entry = L1TxEntry::from_tx(&commit_tx);
            commit_entry.status = L1TxStatus::Published;
            broadcast_db
                .put_tx_entry(commit_txid, commit_entry)
                .expect("store escaped settled commit");
            broadcast_db
                .put_tx_entry(reveal_txid, L1TxEntry::from_tx(&reveal_tx))
                .expect("store unpublished settled reveal");

            let stats = cancel_queued_checkpoint_submissions(
                &storage,
                first_unaccepted_epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("reconcile settled partial escape");

            assert_eq!(stats, WriterCancelStats::default());
            assert_eq!(
                broadcast_db
                    .get_tx_entry_by_id(commit_txid)
                    .expect("read escaped settled commit")
                    .expect("escaped settled commit exists")
                    .status,
                L1TxStatus::Published
            );
            assert_eq!(
                broadcast_db
                    .get_tx_entry_by_id(reveal_txid)
                    .expect("read unpublished settled reveal")
                    .expect("unpublished settled reveal exists")
                    .status,
                L1TxStatus::Unpublished
            );
        }

        #[test]
        fn reconcile_deletes_artifacts_but_leaves_escaped_unpublished_bundle() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let epoch = 3;
            let (checkpoint, l1_payload) = checkpoint_l1_payload(epoch);
            let commitment = checkpoint_commitment(&checkpoint);
            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([10; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let writer_db = db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store checkpoint intent");

            let commit_txid = L1TxId::from([11; 32]);
            let reveal_txid = L1TxId::from([12; 32]);
            let payload_idx = writer_db
                .bundle_intent_payload(
                    intent_id,
                    intent_entry,
                    BundledPayloadEntry::new(
                        l1_payload,
                        commit_txid,
                        reveal_txid,
                        L1BundleStatus::Unpublished,
                    ),
                )
                .expect("bundle checkpoint intent");

            let broadcast_db = db.broadcast_db();
            for txid in [commit_txid, reveal_txid] {
                let mut tx_entry = L1TxEntry::from_tx(&test_transaction());
                tx_entry.status = L1TxStatus::Published;
                broadcast_db
                    .put_tx_entry(Buf32::from(txid.0), tx_entry)
                    .expect("store published transaction");
            }
            storage
                .ol_checkpoint()
                .put_checkpoint_payload_entry_blocking(commitment, checkpoint)
                .expect("store local checkpoint payload");

            let stats = reconcile_unaccepted_checkpoint_artifacts_from_epoch(
                &storage,
                epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("reconcile unaccepted checkpoint");

            assert_eq!(stats.deleted_payloads, 1);
            assert_eq!(
                stats.writer,
                WriterCancelStats {
                    left_published_bundles: 1,
                    ..WriterCancelStats::default()
                }
            );
            assert!(
                storage
                    .ol_checkpoint()
                    .get_checkpoint_payload_entry_blocking(commitment)
                    .expect("read checkpoint payload")
                    .is_none()
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_payload_entry_by_idx_blocking(payload_idx)
                    .expect("read bundle")
                    .expect("bundle exists")
                    .status,
                L1BundleStatus::Unpublished
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read intent")
                    .expect("intent exists")
                    .status,
                IntentStatus::Bundled(payload_idx)
            );
            for txid in [commit_txid, reveal_txid] {
                assert_eq!(
                    broadcast_db
                        .get_tx_entry_by_id(Buf32::from(txid.0))
                        .expect("read broadcast transaction")
                        .expect("broadcast transaction exists")
                        .status,
                    L1TxStatus::Published
                );
            }
        }

        #[test]
        fn cancellation_leaves_partial_escape_entirely_untouched() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let epoch = 4;
            let (checkpoint, l1_payload) = checkpoint_l1_payload(epoch);
            let encoded =
                encode_to_vec(&CodecSsz::new(checkpoint)).expect("encode checkpoint payload");
            let commit_tx = create_dummy_tx(1, 1);
            let mut reveal_tx = create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
            reveal_tx.input[0].previous_output.txid = commit_tx.compute_txid();
            let commit_txid = commit_tx.compute_txid().to_buf32();
            let reveal_txid = reveal_tx.compute_txid().to_buf32();

            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([13; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let writer_db = db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store checkpoint intent");
            let payload_idx = writer_db
                .bundle_intent_payload(
                    intent_id,
                    intent_entry,
                    BundledPayloadEntry::new(
                        l1_payload,
                        L1TxId::from(commit_txid.0),
                        L1TxId::from(reveal_txid.0),
                        L1BundleStatus::Unpublished,
                    ),
                )
                .expect("bundle checkpoint intent");

            let broadcast_db = db.broadcast_db();
            let mut commit_entry = L1TxEntry::from_tx(&commit_tx);
            commit_entry.status = L1TxStatus::Published;
            broadcast_db
                .put_tx_entry(commit_txid, commit_entry)
                .expect("store published commit transaction");
            broadcast_db
                .put_tx_entry(reveal_txid, L1TxEntry::from_tx(&reveal_tx))
                .expect("store unpublished reveal transaction");

            let stats =
                cancel_queued_checkpoint_submissions(&storage, epoch, *TEST_MAGIC_BYTES.as_bytes())
                    .expect("cancel queued checkpoint submissions");

            assert_eq!(
                stats,
                WriterCancelStats {
                    left_published_bundles: 1,
                    ..WriterCancelStats::default()
                }
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_payload_entry_by_idx_blocking(payload_idx)
                    .expect("read bundle")
                    .expect("bundle exists")
                    .status,
                L1BundleStatus::Unpublished
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read intent")
                    .expect("intent exists")
                    .status,
                IntentStatus::Bundled(payload_idx)
            );
            assert_eq!(
                broadcast_db
                    .get_tx_entry_by_id(commit_txid)
                    .expect("read commit transaction")
                    .expect("commit transaction exists")
                    .status,
                L1TxStatus::Published
            );
            assert_eq!(
                broadcast_db
                    .get_tx_entry_by_id(reveal_txid)
                    .expect("read reveal transaction")
                    .expect("reveal transaction exists")
                    .status,
                L1TxStatus::Unpublished
            );
        }

        #[test]
        fn cancellation_relinks_published_orphan_checkpoint_txs() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let epoch = 5;
            let (checkpoint, l1_payload) = checkpoint_l1_payload(epoch);
            let encoded =
                encode_to_vec(&CodecSsz::new(checkpoint)).expect("encode checkpoint payload");
            let commit_tx = create_dummy_tx(1, 1);
            let mut reveal_tx = create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
            reveal_tx.input[0].previous_output.txid = commit_tx.compute_txid();
            let commit_txid = commit_tx.compute_txid().to_buf32();
            let reveal_txid = reveal_tx.compute_txid().to_buf32();

            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([14; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let writer_db = db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store checkpoint intent");
            let payload_idx = writer_db
                .bundle_intent_payload(
                    intent_id,
                    intent_entry,
                    BundledPayloadEntry::new_unsigned(l1_payload),
                )
                .expect("bundle checkpoint intent");

            let broadcast_db = db.broadcast_db();
            for (txid, tx) in [(commit_txid, &commit_tx), (reveal_txid, &reveal_tx)] {
                let mut tx_entry = L1TxEntry::from_tx(tx);
                tx_entry.status = L1TxStatus::Published;
                broadcast_db
                    .put_tx_entry(txid, tx_entry)
                    .expect("store published orphan transaction");
            }

            let stats =
                cancel_queued_checkpoint_submissions(&storage, epoch, *TEST_MAGIC_BYTES.as_bytes())
                    .expect("relink queued checkpoint submission");

            assert_eq!(
                stats,
                WriterCancelStats {
                    relinked_bundles: 1,
                    ..WriterCancelStats::default()
                }
            );
            let bundle = storage
                .l1_writer()
                .get_payload_entry_by_idx_blocking(payload_idx)
                .expect("read bundle")
                .expect("bundle exists");
            assert_eq!(bundle.status, L1BundleStatus::Unpublished);
            assert_eq!(bundle.commit_txid, L1TxId::from(commit_txid.0));
            assert_eq!(bundle.reveal_txid, L1TxId::from(reveal_txid.0));
            assert!(bundle.payload_signature.is_none());
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read intent")
                    .expect("intent exists")
                    .status,
                IntentStatus::Bundled(payload_idx)
            );
            for txid in [commit_txid, reveal_txid] {
                assert_eq!(
                    broadcast_db
                        .get_tx_entry_by_id(txid)
                        .expect("read orphan transaction")
                        .expect("orphan transaction exists")
                        .status,
                    L1TxStatus::Published
                );
            }
        }

        #[test]
        fn cancellation_relinks_bundle_when_only_commit_escaped() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let epoch = 8;
            let (checkpoint, l1_payload) = checkpoint_l1_payload(epoch);
            let encoded =
                encode_to_vec(&CodecSsz::new(checkpoint)).expect("encode checkpoint payload");
            let commit_tx = create_dummy_tx(1, 1);
            let mut reveal_tx = create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
            reveal_tx.input[0].previous_output.txid = commit_tx.compute_txid();
            let commit_txid = commit_tx.compute_txid().to_buf32();
            let reveal_txid = reveal_tx.compute_txid().to_buf32();

            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([15; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let writer_db = db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store checkpoint intent");
            let payload_idx = writer_db
                .bundle_intent_payload(
                    intent_id,
                    intent_entry,
                    BundledPayloadEntry::new_unsigned(l1_payload),
                )
                .expect("bundle checkpoint intent");

            let broadcast_db = db.broadcast_db();
            let mut commit_entry = L1TxEntry::from_tx(&commit_tx);
            commit_entry.status = L1TxStatus::Published;
            broadcast_db
                .put_tx_entry(commit_txid, commit_entry)
                .expect("store published orphan commit");
            broadcast_db
                .put_tx_entry(reveal_txid, L1TxEntry::from_tx(&reveal_tx))
                .expect("store unpublished orphan reveal");

            let stats =
                cancel_queued_checkpoint_submissions(&storage, epoch, *TEST_MAGIC_BYTES.as_bytes())
                    .expect("relink queued checkpoint submission");

            assert_eq!(
                stats,
                WriterCancelStats {
                    relinked_bundles: 1,
                    ..WriterCancelStats::default()
                }
            );
            let bundle = storage
                .l1_writer()
                .get_payload_entry_by_idx_blocking(payload_idx)
                .expect("read bundle")
                .expect("bundle exists");
            assert_eq!(bundle.status, L1BundleStatus::Unpublished);
            assert_eq!(bundle.commit_txid, L1TxId::from(commit_txid.0));
            assert_eq!(bundle.reveal_txid, L1TxId::from(reveal_txid.0));
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read intent")
                    .expect("intent exists")
                    .status,
                IntentStatus::Bundled(payload_idx)
            );
            assert_eq!(
                broadcast_db
                    .get_tx_entry_by_id(reveal_txid)
                    .expect("read orphan reveal")
                    .expect("orphan reveal exists")
                    .status,
                L1TxStatus::Unpublished
            );
            assert_eq!(
                broadcast_db
                    .get_tx_entry_by_id(commit_txid)
                    .expect("read orphan commit")
                    .expect("orphan commit exists")
                    .status,
                L1TxStatus::Published
            );
        }

        #[test]
        fn cancellation_does_not_relink_escaped_checkpoint_from_other_epoch() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test node storage");
            let escaped_epoch = 6;
            let bundle_epoch = 7;
            let (escaped_checkpoint, _) = checkpoint_l1_payload(escaped_epoch);
            let encoded = encode_to_vec(&CodecSsz::new(escaped_checkpoint))
                .expect("encode escaped checkpoint payload");
            let commit_tx = create_dummy_tx(1, 1);
            let mut reveal_tx = create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
            reveal_tx.input[0].previous_output.txid = commit_tx.compute_txid();
            let commit_txid = commit_tx.compute_txid().to_buf32();
            let reveal_txid = reveal_tx.compute_txid().to_buf32();

            let (_, l1_payload) = checkpoint_l1_payload(bundle_epoch);
            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([15; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let writer_db = db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store checkpoint intent");
            let payload_idx = writer_db
                .bundle_intent_payload(
                    intent_id,
                    intent_entry,
                    BundledPayloadEntry::new_unsigned(l1_payload),
                )
                .expect("bundle checkpoint intent");

            let broadcast_db = db.broadcast_db();
            for (txid, tx) in [(commit_txid, &commit_tx), (reveal_txid, &reveal_tx)] {
                let mut tx_entry = L1TxEntry::from_tx(tx);
                tx_entry.status = L1TxStatus::Published;
                broadcast_db
                    .put_tx_entry(txid, tx_entry)
                    .expect("store published orphan transaction");
            }

            let stats = cancel_queued_checkpoint_submissions(
                &storage,
                escaped_epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("cancel mismatched checkpoint submission");

            assert_eq!(
                stats,
                WriterCancelStats {
                    abandoned_intents: 1,
                    abandoned_bundles: 1,
                    ..WriterCancelStats::default()
                }
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_payload_entry_by_idx_blocking(payload_idx)
                    .expect("read bundle")
                    .expect("bundle exists")
                    .status,
                L1BundleStatus::Abandoned
            );
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read intent")
                    .expect("intent exists")
                    .status,
                IntentStatus::Abandoned
            );
            for txid in [commit_txid, reveal_txid] {
                assert_eq!(
                    broadcast_db
                        .get_tx_entry_by_id(txid)
                        .expect("read orphan transaction")
                        .expect("orphan transaction exists")
                        .status,
                    L1TxStatus::Published
                );
            }
        }

        #[test]
        fn cancellation_repairs_checkpoint_intent_with_missing_bundle() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test storage");
            let (_, l1_payload) = checkpoint_l1_payload(3);
            let intent = PayloadIntent::new(PayloadDest::L1, Buf32::from([8; 32]), l1_payload);
            let intent_id = *intent.commitment();
            let mut intent_entry = IntentEntry::new_unbundled(intent);
            intent_entry.status = IntentStatus::Bundled(99);
            let intent_idx = db
                .writer_db()
                .put_intent_entry(intent_id, intent_entry)
                .expect("store orphaned intent");

            let stats =
                cancel_queued_checkpoint_submissions(&storage, 3, *TEST_MAGIC_BYTES.as_bytes())
                    .expect("cancel orphaned intent");

            assert_eq!(stats.repaired_orphans, 1);
            assert_eq!(stats.abandoned_intents, 1);
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read intent")
                    .expect("intent exists")
                    .status,
                IntentStatus::Abandoned
            );
        }

        #[test]
        fn abandoned_bundle_reconcile_is_idempotent() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test storage");
            let (_, l1_payload) = checkpoint_l1_payload(4);
            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([7; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let intent_idx = db
                .writer_db()
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store checkpoint intent");
            let commit_txid = L1TxId::from([3; 32]);
            let reveal_txid = L1TxId::from([4; 32]);
            let bundle = BundledPayloadEntry::new(
                l1_payload,
                commit_txid,
                reveal_txid,
                L1BundleStatus::Abandoned,
            );
            db.writer_db()
                .bundle_intent_payload(intent_id, intent_entry, bundle)
                .expect("store abandoned bundle");
            let broadcast_db = db.broadcast_db();
            for txid in [commit_txid, reveal_txid] {
                broadcast_db
                    .put_tx_entry(Buf32::from(txid.0), L1TxEntry::from_tx(&test_transaction()))
                    .expect("store unpublished transaction");
            }

            let first =
                cancel_queued_checkpoint_submissions(&storage, 4, *TEST_MAGIC_BYTES.as_bytes())
                    .expect("first cancellation pass");
            let second =
                cancel_queued_checkpoint_submissions(&storage, 4, *TEST_MAGIC_BYTES.as_bytes())
                    .expect("second cancellation pass");

            assert_eq!(first.abandoned_intents, 1);
            assert_eq!(first.invalidated_txs, 2);
            assert_eq!(second, WriterCancelStats::default());
            assert_eq!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read intent")
                    .expect("intent exists")
                    .status,
                IntentStatus::Abandoned
            );
        }

        #[test]
        fn published_checkpoint_bundle_is_left_untouched() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test storage");
            let (_, l1_payload) = checkpoint_l1_payload(5);
            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([6; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let intent_idx = db
                .writer_db()
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("store checkpoint intent");
            let payload_idx = db
                .writer_db()
                .bundle_intent_payload(
                    intent_id,
                    intent_entry,
                    BundledPayloadEntry::new(
                        l1_payload,
                        L1TxId::from([5; 32]),
                        L1TxId::from([6; 32]),
                        L1BundleStatus::Published,
                    ),
                )
                .expect("store published bundle");

            let stats =
                cancel_queued_checkpoint_submissions(&storage, 5, *TEST_MAGIC_BYTES.as_bytes())
                    .expect("scan published bundle");

            assert_eq!(stats, WriterCancelStats::default());
            assert_eq!(
                storage
                    .l1_writer()
                    .get_payload_entry_by_idx_blocking(payload_idx)
                    .expect("read bundle")
                    .expect("bundle exists")
                    .status,
                L1BundleStatus::Published
            );
            assert!(matches!(
                storage
                    .l1_writer()
                    .get_intent_by_idx_blocking(intent_idx)
                    .expect("read intent")
                    .expect("intent exists")
                    .status,
                IntentStatus::Bundled(_)
            ));
        }

        #[test]
        fn cancellation_sweeps_unlinked_checkpoint_broadcaster_entry() {
            let db = get_test_sled_backend();
            let storage = create_node_storage(db.clone(), test_runtime_handle())
                .expect("create test storage");
            let epoch = 6;
            let (checkpoint, _) = checkpoint_l1_payload(epoch);
            let encoded =
                encode_to_vec(&CodecSsz::new(checkpoint)).expect("encode checkpoint payload");
            let commit_tx = create_dummy_tx(1, 1);
            let commit_txid = commit_tx.compute_txid().to_buf32();
            let mut checkpoint_tx =
                create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
            checkpoint_tx.input[0].previous_output.txid = commit_tx.compute_txid();
            let non_checkpoint_tx = create_dummy_tx(2, 1);
            let checkpoint_txid = checkpoint_tx.compute_txid().to_buf32();
            let non_checkpoint_txid = non_checkpoint_tx.compute_txid().to_buf32();
            let broadcast_db = db.broadcast_db();
            broadcast_db
                .put_tx_entry(commit_txid, L1TxEntry::from_tx(&commit_tx))
                .expect("store unlinked checkpoint commit entry");
            broadcast_db
                .put_tx_entry(checkpoint_txid, L1TxEntry::from_tx(&checkpoint_tx))
                .expect("store unlinked checkpoint broadcaster entry");
            broadcast_db
                .put_tx_entry(non_checkpoint_txid, L1TxEntry::from_tx(&non_checkpoint_tx))
                .expect("store non-checkpoint broadcaster entry");

            let stats =
                cancel_queued_checkpoint_submissions(&storage, epoch, *TEST_MAGIC_BYTES.as_bytes())
                    .expect("cancel queued checkpoint submissions");

            assert_eq!(stats.invalidated_txs, 2);
            for txid in [commit_txid, checkpoint_txid] {
                assert_eq!(
                    broadcast_db
                        .get_tx_entry_by_id(txid)
                        .expect("read checkpoint broadcaster entry")
                        .expect("checkpoint broadcaster entry exists")
                        .status,
                    L1TxStatus::InvalidInputs
                );
            }
            assert_eq!(
                broadcast_db
                    .get_tx_entry_by_id(non_checkpoint_txid)
                    .expect("read non-checkpoint broadcaster entry")
                    .expect("non-checkpoint broadcaster entry exists")
                    .status,
                L1TxStatus::Unpublished
            );
        }
    }
}
