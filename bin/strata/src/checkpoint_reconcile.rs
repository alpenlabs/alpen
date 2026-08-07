//! Reconciles local checkpoint artifacts against ASM-accepted state.

use anyhow::{Context, Result};
#[cfg(feature = "sequencer")]
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_asm_common::{SectionStateExt, Subprotocol};
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
#[cfg(feature = "sequencer")]
use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
use strata_checkpoint_types::CheckpointProofTask;
#[cfg(feature = "sequencer")]
use strata_codec::decode_buf_exact;
#[cfg(feature = "sequencer")]
use strata_codec_utils::CodecSsz;
#[cfg(feature = "sequencer")]
use strata_csm_types::L1Payload;
use strata_db_types::{
    backend::DatabaseBackend,
    l1_broadcast::{L1BroadcastDatabase, L1TxStatus},
    l1_writer::{IntentStatus, L1BundleStatus, L1WriterDatabase},
    ol_checkpoint::OLCheckpointDatabase,
};
use strata_identifiers::{Epoch, EpochCommitment};
use strata_node_context::NodeContext;
use tracing::{debug, info};

/// Deletes local checkpoint artifacts after ASM's accepted checkpoint tip.
///
/// Checkpoint payloads, proofs, and prover tasks past the ASM verified tip are
/// local candidate state. Rebuilding them on startup prevents a rotated OL
/// image from reusing stale pre-rotation proof artifacts.
pub(crate) fn reconcile_unaccepted_checkpoint_artifacts(nodectx: &NodeContext) -> Result<()> {
    if nodectx.config().prover.is_none() {
        return Ok(());
    }

    let Some(first_unaccepted_epoch) = first_unaccepted_checkpoint_epoch(nodectx)? else {
        return Ok(());
    };

    let storage = nodectx.storage();
    let mut cleanup_commitments =
        checkpoint_commitments_from_epoch(nodectx, first_unaccepted_epoch)?;

    let mut deleted_payloads = Vec::new();
    let mut reconciled_commitments = Vec::new();
    for commitment in cleanup_commitments {
        if !abandon_queued_checkpoint(storage.db().as_ref(), commitment)? {
            continue;
        }

        reconciled_commitments.push(commitment);

        if storage
            .ol_checkpoint()
            .del_local_checkpoint_payload_entry_blocking(commitment)
            .with_context(|| format!("delete unaccepted local checkpoint payload {commitment}"))?
        {
            deleted_payloads.push(commitment);
        }
    }
    cleanup_commitments = reconciled_commitments;

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

    if !deleted_payloads.is_empty() || deleted_proofs > 0 || deleted_tasks > 0 {
        info!(
            first_unaccepted_epoch,
            deleted_payloads = deleted_payloads.len(),
            deleted_proofs,
            deleted_tasks,
            "reconciled unaccepted checkpoint artifacts against ASM verified tip"
        );
    }

    Ok(())
}

/// Abandons the writer and broadcaster entries for a checkpoint before its local artifacts are
/// deleted. Returning `false` preserves the artifacts when the checkpoint intent is not yet
/// bundled, because there is no terminal writer entry to cancel without creating an index hole.
fn abandon_queued_checkpoint(
    db: &impl DatabaseBackend,
    commitment: EpochCommitment,
) -> Result<bool> {
    let Some(intent_idx) = checkpoint_writer_intent_idx(db, commitment)? else {
        return Ok(true);
    };

    let writer_db = db.writer_db();
    let Some(intent) = writer_db
        .get_intent_by_idx(intent_idx)
        .with_context(|| format!("read checkpoint writer intent {intent_idx} for {commitment}"))?
    else {
        anyhow::bail!("checkpoint {commitment} refers to missing writer intent {intent_idx}");
    };

    let IntentStatus::Bundled(payload_idx) = intent.status else {
        debug!(%commitment, intent_idx, "checkpoint intent is not bundled; preserving local artifacts");
        return Ok(false);
    };
    let Some(mut payload) = writer_db
        .get_payload_entry_by_idx(payload_idx)
        .with_context(|| {
            format!("read checkpoint writer payload {payload_idx} for {commitment}")
        })?
    else {
        anyhow::bail!(
            "checkpoint {commitment} intent {intent_idx} refers to missing writer payload {payload_idx}"
        );
    };

    let broadcast_db = db.broadcast_db();
    if is_broadcast_tx_in_flight(broadcast_db.as_ref(), payload.commit_txid.0)?
        || is_broadcast_tx_in_flight(broadcast_db.as_ref(), payload.reveal_txid.0)?
    {
        debug!(%commitment, payload_idx, "checkpoint transaction is already published; preserving local artifacts");
        return Ok(false);
    }
    abandon_broadcast_tx(broadcast_db.as_ref(), payload.commit_txid.0)?;
    abandon_broadcast_tx(broadcast_db.as_ref(), payload.reveal_txid.0)?;

    payload.status = L1BundleStatus::Abandoned;
    writer_db
        .put_payload_entry(payload_idx, payload)
        .with_context(|| {
            format!("abandon checkpoint writer payload {payload_idx} for {commitment}")
        })?;

    Ok(true)
}

fn checkpoint_writer_intent_idx(
    db: &impl DatabaseBackend,
    commitment: EpochCommitment,
) -> Result<Option<u64>> {
    if let Some(intent_idx) = db
        .ol_checkpoint_db()
        .get_checkpoint_signing_entry(commitment)
        .with_context(|| format!("read checkpoint signing entry for {commitment}"))?
    {
        return Ok(Some(intent_idx));
    }

    #[cfg(feature = "sequencer")]
    {
        let writer_db = db.writer_db();
        for intent_idx in 0..writer_db.get_next_intent_idx()? {
            let Some(intent) = writer_db.get_intent_by_idx(intent_idx)? else {
                continue;
            };
            if checkpoint_commitment_from_writer_payload(intent.payload()) == Some(commitment) {
                return Ok(Some(intent_idx));
            }
        }
    }

    Ok(None)
}

#[cfg(feature = "sequencer")]
fn checkpoint_commitment_from_writer_payload(payload: &L1Payload) -> Option<EpochCommitment> {
    let checkpoint_tag = OL_STF_CHECKPOINT_TX_TAG.as_ref();
    if payload.tag().subproto_id() != checkpoint_tag.subproto_id()
        || payload.tag().tx_type() != checkpoint_tag.tx_type()
        || payload.tag().aux_data() != checkpoint_tag.aux_data()
    {
        return None;
    }

    let checkpoint: CodecSsz<CheckpointPayload> =
        decode_buf_exact(&payload.data().concat()).ok()?;
    let checkpoint = checkpoint.into_inner();
    Some(EpochCommitment::from_terminal(
        Epoch::from(checkpoint.new_tip().epoch),
        *checkpoint.new_tip().l2_commitment(),
    ))
}

fn is_broadcast_tx_in_flight(db: &impl L1BroadcastDatabase, txid: [u8; 32]) -> Result<bool> {
    if txid == [0; 32] {
        return Ok(false);
    }

    Ok(matches!(
        db.get_tx_entry_by_id(txid.into())?
            .map(|entry| entry.status),
        Some(L1TxStatus::Published | L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. })
    ))
}

fn abandon_broadcast_tx(db: &impl L1BroadcastDatabase, txid: [u8; 32]) -> Result<()> {
    if txid == [0; 32] {
        return Ok(());
    }

    let Some(mut entry) = db
        .get_tx_entry_by_id(txid.into())
        .context("read queued checkpoint transaction")?
    else {
        return Ok(());
    };
    entry.status = L1TxStatus::Abandoned;
    db.put_tx_entry(txid.into(), entry)
        .context("abandon queued checkpoint transaction")?;
    Ok(())
}

fn checkpoint_commitments_from_epoch(
    nodectx: &NodeContext,
    first_unaccepted_epoch: Epoch,
) -> Result<Vec<EpochCommitment>> {
    let storage = nodectx.storage();
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
    use bitcoin::{Transaction, absolute, transaction};
    use strata_asm_checkpoint_types::test_utils::create_test_checkpoint_payload;
    use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
    use strata_codec::encode_to_vec;
    use strata_codec_utils::CodecSsz;
    use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::{
        backend::DatabaseBackend,
        common::L1TxId,
        l1_broadcast::{L1BroadcastDatabase, L1TxEntry, L1TxStatus},
        l1_writer::{
            BundledPayloadEntry, IntentEntry, IntentStatus, L1BundleStatus, L1WriterDatabase,
        },
        ol_checkpoint::OLCheckpointDatabase,
    };
    use strata_identifiers::{Buf32, Epoch, EpochCommitment};
    use strata_l1_txfmt::TagData;

    use super::abandon_queued_checkpoint;

    fn checkpoint_commitment(epoch: u32) -> EpochCommitment {
        let checkpoint = create_test_checkpoint_payload(epoch);
        EpochCommitment::from_terminal(Epoch::from(epoch), *checkpoint.new_tip().l2_commitment())
    }

    fn test_transaction() -> Transaction {
        Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: Vec::new(),
            output: Vec::new(),
        }
    }

    fn checkpoint_writer_payload(epoch: u32) -> L1Payload {
        let encoded = encode_to_vec(&CodecSsz::new(create_test_checkpoint_payload(epoch)))
            .expect("test: encode checkpoint payload");
        L1Payload::new(vec![encoded], OL_STF_CHECKPOINT_TX_TAG.clone())
            .expect("test: build checkpoint writer payload")
    }

    #[test]
    fn reconciliation_abandons_queued_checkpoint_without_deleting_queue_indices() {
        let db = get_test_sled_backend();
        let commitment = checkpoint_commitment(14);
        db.ol_checkpoint_db()
            .put_checkpoint_payload_entry(commitment, create_test_checkpoint_payload(14))
            .expect("test: store checkpoint payload");

        let payload = L1Payload::new(
            vec![vec![1, 2, 3]],
            TagData::new(1, 1, vec![]).expect("test tag is valid"),
        )
        .expect("test payload is valid");
        let intent = PayloadIntent::new(PayloadDest::L1, Buf32::from([3; 32]), payload.clone());
        let payload_idx = 0;
        let commit_txid = L1TxId::from([4; 32]);
        let reveal_txid = L1TxId::from([5; 32]);
        db.writer_db()
            .put_payload_entry(
                payload_idx,
                BundledPayloadEntry::new(
                    payload,
                    commit_txid,
                    reveal_txid,
                    L1BundleStatus::Unpublished,
                ),
            )
            .expect("test: store writer payload");
        let intent_idx = db
            .writer_db()
            .put_intent_entry(
                *intent.commitment(),
                IntentEntry::new_bundled(intent, payload_idx),
            )
            .expect("test: store writer intent");
        db.ol_checkpoint_db()
            .put_checkpoint_signing_entry(commitment, intent_idx)
            .expect("test: store checkpoint signing entry");

        for txid in [commit_txid, reveal_txid] {
            db.broadcast_db()
                .put_tx_entry(Buf32(txid.0), L1TxEntry::from_tx(&test_transaction()))
                .expect("test: store broadcast transaction");
        }

        assert!(abandon_queued_checkpoint(db.as_ref(), commitment).expect("test: abandon queue"));
        assert!(
            db.ol_checkpoint_db()
                .del_local_checkpoint_payload_entry(commitment)
                .expect("test: delete local payload")
        );

        assert_eq!(
            db.writer_db()
                .get_payload_entry_by_idx(payload_idx)
                .expect("test: get writer payload")
                .expect("test: writer payload exists")
                .status,
            L1BundleStatus::Abandoned
        );
        assert_eq!(
            db.writer_db()
                .get_intent_by_idx(intent_idx)
                .expect("test: get writer intent")
                .expect("test: writer intent exists")
                .status,
            IntentStatus::Bundled(payload_idx)
        );
        for txid in [commit_txid, reveal_txid] {
            assert_eq!(
                db.broadcast_db()
                    .get_tx_entry_by_id(Buf32(txid.0))
                    .expect("test: get broadcast transaction")
                    .expect("test: broadcast transaction exists")
                    .status,
                L1TxStatus::Abandoned
            );
        }
    }

    #[test]
    fn reconciliation_preserves_checkpoint_already_published_to_l1() {
        let db = get_test_sled_backend();
        let commitment = checkpoint_commitment(14);
        db.ol_checkpoint_db()
            .put_checkpoint_payload_entry(commitment, create_test_checkpoint_payload(14))
            .expect("test: store checkpoint payload");
        let payload = L1Payload::new(
            vec![vec![1, 2, 3]],
            TagData::new(1, 1, vec![]).expect("test tag is valid"),
        )
        .expect("test payload is valid");
        let commit_txid = L1TxId::from([4; 32]);
        db.writer_db()
            .put_payload_entry(
                0,
                BundledPayloadEntry::new(
                    payload.clone(),
                    commit_txid,
                    L1TxId::zero(),
                    L1BundleStatus::Unpublished,
                ),
            )
            .expect("test: store writer payload");
        let intent_idx = db
            .writer_db()
            .put_intent_entry(
                Buf32::from([3; 32]),
                IntentEntry::new_bundled(
                    PayloadIntent::new(PayloadDest::L1, Buf32::from([3; 32]), payload),
                    0,
                ),
            )
            .expect("test: store writer intent");
        db.ol_checkpoint_db()
            .put_checkpoint_signing_entry(commitment, intent_idx)
            .expect("test: store checkpoint signing entry");
        let mut published = L1TxEntry::from_tx(&test_transaction());
        published.status = L1TxStatus::Published;
        db.broadcast_db()
            .put_tx_entry(Buf32(commit_txid.0), published)
            .expect("test: store published transaction");

        assert!(!abandon_queued_checkpoint(db.as_ref(), commitment).expect("test: reconcile"));
        assert_eq!(
            db.writer_db()
                .get_payload_entry_by_idx(0)
                .expect("test: get writer payload")
                .expect("test: writer payload exists")
                .status,
            L1BundleStatus::Unpublished
        );
        assert!(
            db.ol_checkpoint_db()
                .get_checkpoint_signing_entry(commitment)
                .expect("test: get signing entry")
                .is_some()
        );
    }

    #[test]
    fn reconciliation_recovers_checkpoint_intent_missing_its_signing_marker() {
        let db = get_test_sled_backend();
        let commitment = checkpoint_commitment(14);
        db.ol_checkpoint_db()
            .put_checkpoint_payload_entry(commitment, create_test_checkpoint_payload(14))
            .expect("test: store checkpoint payload");
        let payload = checkpoint_writer_payload(14);
        let intent = PayloadIntent::new(PayloadDest::L1, Buf32::from([3; 32]), payload.clone());
        db.writer_db()
            .put_payload_entry(0, BundledPayloadEntry::new_unsigned(payload))
            .expect("test: store writer payload");
        db.writer_db()
            .put_intent_entry(*intent.commitment(), IntentEntry::new_bundled(intent, 0))
            .expect("test: store writer intent");

        assert!(abandon_queued_checkpoint(db.as_ref(), commitment).expect("test: reconcile"));
        assert_eq!(
            db.writer_db()
                .get_payload_entry_by_idx(0)
                .expect("test: get writer payload")
                .expect("test: writer payload exists")
                .status,
            L1BundleStatus::Abandoned
        );
    }
}
