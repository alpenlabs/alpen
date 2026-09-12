//! Reconciles local checkpoint artifacts against ASM-accepted state.

use std::collections::HashMap;

use anyhow::{Context, Result};
use strata_db_types::l1_broadcast::L1TxStatus;
use strata_db_types::l1_writer::{BundledPayloadEntry, IntentEntry, IntentStatus, L1BundleStatus};
use strata_identifiers::{Epoch, EpochCommitment};
use tracing::info;

use super::context::{CheckpointContextResult, CheckpointReconcileContext};
use super::publication::checkpoint_from_payload;

/// Reconciles local checkpoint artifacts after ASM's accepted checkpoint tip.
///
/// Checkpoint payloads, proofs, and prover tasks past the ASM verified tip are
/// local candidate state. Rebuilding them on startup prevents a rotated OL
/// image from reusing stale pre-rotation proof artifacts. Artifacts are deleted
/// only when every corresponding writer intent can be safely abandoned; live or
/// ambiguous publication state preserves both halves for recovery.
pub fn reconcile_unaccepted_checkpoint_artifacts(
    context: &impl CheckpointReconcileContext,
) -> Result<()> {
    let Some(first_unaccepted_epoch) = context.get_first_unaccepted_checkpoint_epoch()? else {
        return Ok(());
    };

    let commitments = checkpoint_commitments_from_epoch(context, first_unaccepted_epoch)?;
    if commitments.is_empty() {
        return Ok(());
    }
    let Some(checkpoint_intents) = checkpoint_intents_by_commitment(context)? else {
        return Ok(());
    };
    let mut deleted_payloads = Vec::new();
    let mut deleted_proofs = 0usize;
    let mut deleted_tasks = 0usize;
    for commitment in commitments {
        if !cancel_queued_checkpoint(
            context,
            checkpoint_intents
                .get(&commitment)
                .map(Vec::as_slice)
                .unwrap_or_default(),
        )? {
            continue;
        }

        if context
            .delete_checkpoint_proof(commitment)
            .with_context(|| format!("delete checkpoint proof for commitment {commitment}"))?
        {
            deleted_proofs += 1;
        }

        if context
            .delete_checkpoint_prover_task(commitment)
            .with_context(|| format!("delete checkpoint prover task for commitment {commitment}"))?
        {
            deleted_tasks += 1;
        }

        if context.delete_unobserved_checkpoint_payload(commitment)? {
            deleted_payloads.push(commitment);
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

/// Collects locally known checkpoint commitments at or after the given epoch.
///
/// This combines checkpoint payload records with summarized epoch state because
/// either side may survive an interrupted write or an earlier cleanup.
fn checkpoint_commitments_from_epoch(
    context: &impl CheckpointReconcileContext,
    first_unaccepted_epoch: Epoch,
) -> Result<Vec<EpochCommitment>> {
    let mut commitments = context
        .get_checkpoint_payload_commitments_from_epoch(first_unaccepted_epoch)
        .context("read unaccepted checkpoint payload commitments")?;
    let Some(last_summarized_epoch) = context
        .get_last_summarized_epoch()
        .context("read last summarized checkpoint epoch")?
    else {
        return Ok(commitments);
    };

    if first_unaccepted_epoch > last_summarized_epoch {
        return Ok(commitments);
    }

    for epoch in first_unaccepted_epoch..=last_summarized_epoch {
        let epoch_commitments = context
            .get_epoch_commitments_at(epoch)
            .with_context(|| format!("read checkpoint commitments for epoch {epoch}"))?;
        extend_missing(&mut commitments, epoch_commitments);
    }

    Ok(commitments)
}

/// Groups checkpoint writer intents by commitment with one database scan.
///
/// A malformed checkpoint payload returns `None`, conservatively disabling the
/// cleanup because its relationship to the candidate artifacts is unknown.
fn checkpoint_intents_by_commitment(
    context: &impl CheckpointReconcileContext,
) -> Result<Option<HashMap<EpochCommitment, Vec<IntentEntry>>>> {
    let mut grouped = HashMap::<EpochCommitment, Vec<IntentEntry>>::new();
    for idx in 0..context.get_next_intent_idx()? {
        let Some(intent) = context.get_intent_by_idx(idx)? else {
            continue;
        };
        let payload = match checkpoint_from_payload(intent.payload()) {
            Ok(Some(payload)) => payload,
            Ok(None) => continue,
            Err(()) => return Ok(None),
        };
        let commitment = EpochCommitment::from_terminal(
            Epoch::from(payload.new_tip().epoch),
            *payload.new_tip().l2_commitment(),
        );
        grouped.entry(commitment).or_default().push(intent);
    }
    Ok(Some(grouped))
}

/// Cancels every supplied writer intent before its checkpoint artifacts are deleted.
///
/// Returns `true` only when every matching intent is safely terminalized. A
/// `false` result preserves the checkpoint artifacts; independently safe intents
/// may already have been terminalized.
fn cancel_queued_checkpoint(
    context: &impl CheckpointReconcileContext,
    intents: &[IntentEntry],
) -> Result<bool> {
    let mut all_cancelled = true;
    for intent in intents {
        all_cancelled &= cancel_writer_intent(context, intent.clone())?;
    }
    Ok(all_cancelled)
}

/// Abandons one writer intent only when no associated transaction may be live on L1.
///
/// Unbundled intents are terminalized directly. Bundled intents are preserved if
/// publication has started for either broadcaster entry.
fn cancel_writer_intent(
    context: &impl CheckpointReconcileContext,
    intent: IntentEntry,
) -> Result<bool> {
    let IntentStatus::Bundled {
        bundle_idx: payload_idx,
    } = intent.status
    else {
        let mut payload = BundledPayloadEntry::new_unsigned(intent.payload().clone());
        payload.status = L1BundleStatus::Abandoned;
        context.abandon_unbundled_intent(intent, payload)?;
        return Ok(true);
    };
    let mut payload = context
        .get_payload_by_idx(payload_idx)?
        .with_context(|| format!("missing checkpoint writer payload {payload_idx}"))?;
    if payload.status.has_reached_l1() {
        return Ok(false);
    }
    let entries = [payload.commit_txid.0, payload.reveal_txid.0]
        .into_iter()
        .map(|txid| {
            context
                .get_broadcast_entry_by_id(txid.into())
                .map(|entry| (txid, entry))
        })
        .collect::<CheckpointContextResult<Vec<_>>>()?;
    if entries
        .iter()
        .filter_map(|(_, entry)| entry.as_ref())
        .any(|entry| entry.status.submission_started())
    {
        return Ok(false);
    }
    for (txid, entry) in entries {
        if let Some(mut entry) = entry {
            entry.status = L1TxStatus::Abandoned;
            context.put_broadcast_entry(txid.into(), entry)?;
        }
    }
    payload.status = L1BundleStatus::Abandoned;
    context.put_payload(payload_idx, payload)?;
    Ok(true)
}

/// Appends candidates that are not already present while preserving discovery order.
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

#[cfg(test)]
mod tests {
    use bitcoin::{absolute, transaction, Transaction};
    use std::sync::Arc;
    use strata_asm_checkpoint_types::test_utils::create_test_checkpoint_payload;
    use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
    use strata_btcio::L1TxEntryExt;
    use strata_codec::encode_to_vec;
    use strata_codec_utils::CodecSsz;
    use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_store_sled::SledBackend;
    use strata_db_types::backend::DatabaseBackend;
    use strata_db_types::common::L1TxId;
    use strata_db_types::l1_broadcast::{L1BroadcastDatabase, L1TxEntry, L1TxStatus};
    use strata_db_types::l1_writer::{
        BundledPayloadEntry, IntentEntry, L1BundleStatus, L1WriterDatabase,
    };
    use strata_db_types::ol_checkpoint::OLCheckpointDatabase;
    use strata_identifiers::{Buf32, Epoch, EpochCommitment};

    use super::{cancel_queued_checkpoint, checkpoint_intents_by_commitment};
    use crate::checkpoint::{CheckpointContextResult, CheckpointReconcileContext};

    struct TestReconcileContext {
        db: Arc<SledBackend>,
    }

    impl TestReconcileContext {
        fn new() -> Self {
            Self {
                db: get_test_sled_backend(),
            }
        }
    }

    impl CheckpointReconcileContext for TestReconcileContext {
        fn get_first_unaccepted_checkpoint_epoch(&self) -> CheckpointContextResult<Option<Epoch>> {
            Ok(None)
        }

        fn get_checkpoint_payload_commitments_from_epoch(
            &self,
            epoch: Epoch,
        ) -> CheckpointContextResult<Vec<EpochCommitment>> {
            Ok(self
                .db
                .ol_checkpoint_db()
                .get_checkpoint_payload_commitments_from_epoch(epoch)?)
        }

        fn get_last_summarized_epoch(&self) -> CheckpointContextResult<Option<Epoch>> {
            Ok(self.db.ol_checkpoint_db().get_last_summarized_epoch()?)
        }

        fn get_epoch_commitments_at(
            &self,
            epoch: Epoch,
        ) -> CheckpointContextResult<Vec<EpochCommitment>> {
            Ok(self.db.ol_checkpoint_db().get_epoch_commitments_at(epoch)?)
        }

        fn get_next_intent_idx(&self) -> CheckpointContextResult<u64> {
            Ok(self.db.writer_db().get_next_intent_idx()?)
        }

        fn get_intent_by_idx(&self, idx: u64) -> CheckpointContextResult<Option<IntentEntry>> {
            Ok(self.db.writer_db().get_intent_by_idx(idx)?)
        }

        fn abandon_unbundled_intent(
            &self,
            intent: IntentEntry,
            payload: BundledPayloadEntry,
        ) -> CheckpointContextResult<()> {
            let commitment = *intent.intent.commitment();
            self.db
                .writer_db()
                .bundle_intent_payload(commitment, intent, payload)?;
            Ok(())
        }

        fn get_payload_by_idx(
            &self,
            idx: u64,
        ) -> CheckpointContextResult<Option<BundledPayloadEntry>> {
            Ok(self.db.writer_db().get_payload_entry_by_idx(idx)?)
        }

        fn put_payload(
            &self,
            idx: u64,
            payload: BundledPayloadEntry,
        ) -> CheckpointContextResult<()> {
            Ok(self.db.writer_db().put_payload_entry(idx, payload)?)
        }

        fn get_broadcast_entry_by_id(
            &self,
            txid: Buf32,
        ) -> CheckpointContextResult<Option<L1TxEntry>> {
            Ok(self.db.broadcast_db().get_tx_entry_by_id(txid)?)
        }

        fn put_broadcast_entry(
            &self,
            txid: Buf32,
            entry: L1TxEntry,
        ) -> CheckpointContextResult<()> {
            self.db.broadcast_db().put_tx_entry(txid, entry)?;
            Ok(())
        }

        fn delete_checkpoint_proof(&self, _: EpochCommitment) -> CheckpointContextResult<bool> {
            Ok(false)
        }

        fn delete_checkpoint_prover_task(
            &self,
            _: EpochCommitment,
        ) -> CheckpointContextResult<bool> {
            Ok(false)
        }

        fn delete_unobserved_checkpoint_payload(
            &self,
            commitment: EpochCommitment,
        ) -> CheckpointContextResult<bool> {
            Ok(self
                .db
                .ol_checkpoint_db()
                .del_local_checkpoint_payload_if_unobserved(commitment)?)
        }
    }

    #[test]
    fn queued_unsubmitted_checkpoint_is_cancelled() {
        let context = TestReconcileContext::new();
        let db = &context.db;
        let checkpoint = create_test_checkpoint_payload(14);
        let commitment = EpochCommitment::from_terminal(
            Epoch::from(14u32),
            *checkpoint.new_tip().l2_commitment(),
        );
        let encoded = encode_to_vec(&CodecSsz::new(checkpoint.clone())).unwrap();
        let payload = L1Payload::new(vec![encoded], OL_STF_CHECKPOINT_TX_TAG.clone()).unwrap();
        let intent = PayloadIntent::new(PayloadDest::L1, Buf32::from([3; 32]), payload.clone());
        let (commit_txid, reveal_txid) = (L1TxId::from([4; 32]), L1TxId::from([5; 32]));
        db.writer_db()
            .put_payload_entry(
                0,
                BundledPayloadEntry::new(
                    payload,
                    commit_txid,
                    reveal_txid,
                    L1BundleStatus::Unpublished,
                ),
            )
            .unwrap();
        db.writer_db()
            .put_intent_entry(*intent.commitment(), IntentEntry::new_bundled(intent, 0))
            .unwrap();

        let tx = Transaction {
            version: transaction::Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![],
            output: vec![],
        };
        for txid in [commit_txid, reveal_txid] {
            db.broadcast_db()
                .put_tx_entry(Buf32(txid.0), L1TxEntry::from_tx(&tx))
                .unwrap();
        }

        let intents = checkpoint_intents_by_commitment(&context).unwrap().unwrap();
        let matching = intents.get(&commitment).unwrap();

        let mut submitting = db
            .broadcast_db()
            .get_tx_entry_by_id(Buf32(commit_txid.0))
            .unwrap()
            .unwrap();
        submitting.status = L1TxStatus::Submitting;
        db.broadcast_db()
            .put_tx_entry(Buf32(commit_txid.0), submitting.clone())
            .unwrap();
        assert!(!cancel_queued_checkpoint(&context, matching).unwrap());
        submitting.status = L1TxStatus::Unpublished;
        db.broadcast_db()
            .put_tx_entry(Buf32(commit_txid.0), submitting.clone())
            .unwrap();
        assert!(!cancel_queued_checkpoint(&context, matching).unwrap());
        submitting.status = L1TxStatus::Queued;
        db.broadcast_db()
            .put_tx_entry(Buf32(commit_txid.0), submitting)
            .unwrap();

        assert!(cancel_queued_checkpoint(&context, matching).unwrap());
        assert_eq!(
            db.writer_db()
                .get_payload_entry_by_idx(0)
                .unwrap()
                .unwrap()
                .status,
            L1BundleStatus::Abandoned
        );
        for txid in [commit_txid, reveal_txid] {
            assert_eq!(
                db.broadcast_db()
                    .get_tx_entry_by_id(Buf32(txid.0))
                    .unwrap()
                    .unwrap()
                    .status,
                L1TxStatus::Abandoned
            );
        }
    }
}
