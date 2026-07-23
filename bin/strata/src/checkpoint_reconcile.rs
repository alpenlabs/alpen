//! Reconciles local checkpoint artifacts against ASM-accepted state.

use std::{fmt, sync::Arc};

use anyhow::{Context, Result};
use strata_asm_common::{SectionStateExt, Subprotocol};
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
use strata_btcio::writer::{CheckpointFailureHandler, PayloadCheckpointRef};
use strata_checkpoint_types::CheckpointProofTask;
use strata_identifiers::{Epoch, EpochCommitment};
use strata_node_context::NodeContext;
use strata_ol_checkpoint::{
    l1_tx::checkpoint_payload_id,
    reconcile::{
        WriterCancelStats, cancel_queued_checkpoint_submissions,
        cancel_settled_checkpoint_submissions,
    },
};
use strata_storage::NodeStorage;
use tracing::{debug, info, warn};

/// Clears a stale checkpoint signing marker after its retiring envelope fails.
pub(crate) struct CheckpointFailureCleanup {
    storage: Arc<NodeStorage>,
}

impl fmt::Debug for CheckpointFailureCleanup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CheckpointFailureCleanup")
            .finish_non_exhaustive()
    }
}

impl CheckpointFailureCleanup {
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }
}

impl CheckpointFailureHandler for CheckpointFailureCleanup {
    fn handle_failed_checkpoint(&self, checkpoint: PayloadCheckpointRef) -> Result<()> {
        let PayloadCheckpointRef::Checkpoint { epoch, id } = checkpoint else {
            return Ok(());
        };
        let checkpoint_db = self.storage.ol_checkpoint();
        let Some(commitment) = checkpoint_db
            .get_canonical_epoch_commitment_at_blocking(epoch)
            .context("resolve failed checkpoint commitment")?
        else {
            warn!(epoch, %id, "failed checkpoint has no canonical epoch commitment");
            return Ok(());
        };
        let Some(payload) = checkpoint_db
            .get_checkpoint_payload_entry_blocking(commitment)
            .context("read failed checkpoint payload")?
        else {
            return Ok(());
        };
        let current_id = checkpoint_payload_id(&payload);

        if current_id != id {
            debug!(
                epoch,
                %id,
                %current_id,
                "failed retiring envelope differs from current checkpoint candidate"
            );
            return Ok(());
        }

        if checkpoint_db
            .del_checkpoint_signing_entry_blocking(commitment)
            .context("clear failed checkpoint signing marker")?
        {
            info!(epoch, %id, "cleared signing marker for failed retiring checkpoint");
        }

        Ok(())
    }
}

/// Reconciles checkpoint queue state against ASM's accepted checkpoint tip.
///
/// Every node cancels stale submissions for already-settled epochs. Nodes with
/// a local prover also cancel and rebuild submissions past the ASM verified tip
/// so a rotated OL image cannot reuse stale pre-rotation proof artifacts.
pub(crate) fn reconcile_unaccepted_checkpoint_artifacts(nodectx: &NodeContext) -> Result<()> {
    let Some(first_unaccepted_epoch) = first_unaccepted_checkpoint_epoch(nodectx)? else {
        return Ok(());
    };
    let last_settled_epoch = last_declared_final_checkpoint_epoch(nodectx)?;

    let storage = nodectx.storage();
    let magic_bytes = *nodectx.asm_params().magic.as_bytes();
    let prover_configured = nodectx.config().prover.is_some();
    let stats = if prover_configured {
        reconcile_unaccepted_checkpoint_artifacts_from_epoch(
            storage,
            last_settled_epoch,
            first_unaccepted_epoch,
            magic_bytes,
        )?
    } else {
        // Without a prover nothing rebuilds a cancelled checkpoint, so only the
        // settled side is safe to touch: those epochs are already accepted and
        // their queued duplicates are the ones that republish on startup.
        ReconcileStats {
            writer: cancel_settled_checkpoint_submissions(
                storage,
                last_settled_epoch,
                magic_bytes,
            )?,
            ..ReconcileStats::default()
        }
    };

    if stats.has_changes() {
        info!(
            ?last_settled_epoch,
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
            "reconciled checkpoint queue and local artifacts against finalized and verified tips"
        );
    }

    Ok(())
}

/// Cancels queued checkpoint submissions that the client has since declared final.
///
/// This pass is safe after checkpoint production starts because it does not delete
/// artifacts for verified-but-reorgable or unaccepted checkpoints.
#[cfg(feature = "sequencer")]
pub(crate) fn reconcile_settled_checkpoint_queue(nodectx: &NodeContext) -> Result<()> {
    let last_settled_epoch = last_declared_final_checkpoint_epoch(nodectx)?;
    let stats = cancel_settled_checkpoint_submissions(
        nodectx.storage(),
        last_settled_epoch,
        *nodectx.asm_params().magic.as_bytes(),
    )?;

    if stats != WriterCancelStats::default() {
        info!(
            ?last_settled_epoch,
            abandoned_intents = stats.abandoned_intents,
            abandoned_bundles = stats.abandoned_bundles,
            left_published_bundles = stats.left_published_bundles,
            relinked_bundles = stats.relinked_bundles,
            invalidated_txs = stats.invalidated_txs,
            repaired_orphans = stats.repaired_orphans,
            "reconciled settled checkpoint queue before starting the broadcaster"
        );
    }

    Ok(())
}

/// Cancels the queued submissions from `first_unaccepted_epoch` on and deletes the
/// local artifacts behind them, so the checkpoint worker rebuilds those epochs.
fn reconcile_unaccepted_checkpoint_artifacts_from_epoch(
    storage: &NodeStorage,
    last_settled_epoch: Option<Epoch>,
    first_unaccepted_epoch: Epoch,
    magic_bytes: [u8; 4],
) -> Result<ReconcileStats> {
    let cancel_stats = cancel_queued_checkpoint_submissions(
        storage,
        last_settled_epoch,
        first_unaccepted_epoch,
        magic_bytes,
    )?;
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

/// Collects the checkpoint commitments summarized from `first_unaccepted_epoch` on.
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

/// What a reconciliation pass changed, for the startup log line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ReconcileStats {
    /// Queue changes reported by the writer cancellation pass.
    writer: WriterCancelStats,
    /// Local checkpoint payload entries deleted.
    deleted_payloads: usize,
    /// Checkpoint proofs deleted.
    deleted_proofs: usize,
    /// Prover tasks deleted.
    deleted_tasks: usize,
}

impl ReconcileStats {
    /// Returns whether the pass changed anything worth logging.
    fn has_changes(self) -> bool {
        self != Self::default()
    }
}

/// Appends the candidates that `items` does not already hold.
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

/// Resolves the first epoch ASM has not verified yet, if ASM state is available.
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

/// Resolves the latest checkpoint epoch the client has declared final.
fn last_declared_final_checkpoint_epoch(nodectx: &NodeContext) -> Result<Option<Epoch>> {
    let latest_client_state = nodectx
        .storage()
        .client_state()
        .fetch_most_recent_state()
        .context("fetch most recent client state for checkpoint reconciliation")?;

    Ok(latest_client_state
        .and_then(|(_, state)| state.get_declared_final_epoch())
        .map(|commitment| commitment.epoch))
}

#[cfg(all(test, feature = "sequencer"))]
mod tests {
    use std::sync::Arc;

    use bitcoin::{Transaction, absolute::LockTime, transaction::Version};
    use strata_asm_checkpoint_types::{
        CheckpointPayload, test_utils::create_test_checkpoint_payload,
    };
    use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
    use strata_asm_proto_txs_test_utils::{
        TEST_MAGIC_BYTES, create_dummy_tx, create_reveal_transaction_stub,
    };
    use strata_btc_types::TxidExt;
    use strata_checkpoint_types::EpochSummary;
    use strata_codec::encode_to_vec;
    use strata_codec_utils::CodecSsz;
    use strata_csm_types::{L1Payload, PayloadDest, PayloadIntent};
    use strata_db_store_sled::{SledBackend, test_utils::get_test_sled_backend};
    use strata_db_types::{
        backend::DatabaseBackend,
        common::L1TxId,
        l1_broadcast::{L1BroadcastDatabase, L1TxEntry, L1TxStatus},
        l1_writer::{
            BundledPayloadEntry, IntentEntry, IntentStatus, L1BundleStatus, L1WriterDatabase,
        },
    };
    use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, OLBlockCommitment};
    use strata_storage::{create_node_storage, test_runtime_handle};

    use super::*;

    /// How the envelope transactions backing a queued checkpoint are stored.
    #[derive(Clone)]
    enum EnvelopeTxs {
        /// Entries the broadcaster sweep cannot decode, so only the bundle link
        /// reaches them.
        Opaque(L1TxStatus, L1TxStatus),
        /// A tagged commit/reveal pair the sweep can decode.
        Decodable(L1TxStatus, L1TxStatus),
    }

    /// A checkpoint queued in the writer DB, with the transactions backing it.
    struct QueuedCheckpoint {
        commitment: EpochCommitment,
        checkpoint: CheckpointPayload,
        intent_idx: u64,
        payload_idx: u64,
        commit_txid: Buf32,
        reveal_txid: Buf32,
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

        /// Queues a checkpoint for `epoch` as a bundle at `bundle_status`, backed by
        /// `envelope`. `seed` keeps fixtures within one test distinct.
        fn queue_checkpoint(
            &self,
            epoch: Epoch,
            seed: u8,
            bundle_status: L1BundleStatus,
            envelope: EnvelopeTxs,
        ) -> QueuedCheckpoint {
            let checkpoint = create_test_checkpoint_payload(epoch);
            let commitment = EpochCommitment::from_terminal(
                checkpoint.new_tip().epoch,
                *checkpoint.new_tip().l2_commitment(),
            );
            let encoded = encode_to_vec(&CodecSsz::new(checkpoint.clone()))
                .expect("test: encode checkpoint payload");
            let l1_payload =
                L1Payload::new(vec![encoded.clone()], OL_STF_CHECKPOINT_TX_TAG.clone())
                    .expect("test: build L1 checkpoint payload");
            let (commit_txid, reveal_txid) = self.put_envelope_txs(encoded, seed, envelope);

            let intent =
                PayloadIntent::new(PayloadDest::L1, Buf32::from([seed; 32]), l1_payload.clone());
            let intent_id = *intent.commitment();
            let intent_entry = IntentEntry::new_unbundled(intent);
            let writer_db = self.db.writer_db();
            let intent_idx = writer_db
                .put_intent_entry(intent_id, intent_entry.clone())
                .expect("test: store checkpoint intent");
            let payload_idx = writer_db
                .bundle_intent_payload(
                    intent_id,
                    intent_entry,
                    BundledPayloadEntry::new(
                        l1_payload,
                        L1TxId::from(commit_txid.0),
                        L1TxId::from(reveal_txid.0),
                        bundle_status,
                    ),
                )
                .expect("test: bundle checkpoint intent");

            QueuedCheckpoint {
                commitment,
                checkpoint,
                intent_idx,
                payload_idx,
                commit_txid,
                reveal_txid,
            }
        }

        /// Stores the broadcaster entries for a queued checkpoint.
        fn put_envelope_txs(
            &self,
            encoded: Vec<u8>,
            seed: u8,
            envelope: EnvelopeTxs,
        ) -> (Buf32, Buf32) {
            let (commit_txid, commit_tx, commit_status, reveal_txid, reveal_tx, reveal_status) =
                match envelope {
                    // Opaque transactions are identical, so their ids are forced apart.
                    EnvelopeTxs::Opaque(commit_status, reveal_status) => (
                        Buf32::from([seed; 32]),
                        opaque_transaction(),
                        commit_status,
                        Buf32::from([seed.wrapping_add(1); 32]),
                        opaque_transaction(),
                        reveal_status,
                    ),
                    EnvelopeTxs::Decodable(commit_status, reveal_status) => {
                        let commit_tx = create_dummy_tx(seed as usize, 1);
                        let mut reveal_tx =
                            create_reveal_transaction_stub(encoded, &OL_STF_CHECKPOINT_TX_TAG);
                        reveal_tx.input[0].previous_output.txid = commit_tx.compute_txid();
                        (
                            commit_tx.compute_txid().to_buf32(),
                            commit_tx,
                            commit_status,
                            reveal_tx.compute_txid().to_buf32(),
                            reveal_tx,
                            reveal_status,
                        )
                    }
                };

            let broadcast_db = self.db.broadcast_db();
            for (txid, tx, status) in [
                (commit_txid, &commit_tx, commit_status),
                (reveal_txid, &reveal_tx, reveal_status),
            ] {
                let mut tx_entry = L1TxEntry::from_tx(tx);
                tx_entry.status = status;
                broadcast_db
                    .put_tx_entry(txid, tx_entry)
                    .expect("test: store broadcaster transaction");
            }

            (commit_txid, reveal_txid)
        }

        /// Stores the local checkpoint payload artifact, and optionally its signing marker.
        fn put_artifacts(&self, queued: &QueuedCheckpoint, signing_marker: bool) {
            self.storage
                .ol_checkpoint()
                .put_checkpoint_payload_entry_blocking(queued.commitment, queued.checkpoint.clone())
                .expect("test: store local checkpoint payload");
            if signing_marker {
                self.storage
                    .ol_checkpoint()
                    .put_checkpoint_signing_entry_blocking(queued.commitment, queued.intent_idx)
                    .expect("test: store checkpoint signing marker");
            }
        }

        fn put_epoch_summary(&self, queued: &QueuedCheckpoint) {
            let summary = EpochSummary::new(
                queued.commitment.epoch,
                *queued.checkpoint.new_tip().l2_commitment(),
                OLBlockCommitment::null(),
                L1BlockCommitment::new(0, L1BlockId::default()),
                Buf32::zero(),
            );
            self.storage
                .ol_checkpoint()
                .insert_epoch_summary_blocking(summary)
                .expect("test: store epoch summary");
        }

        fn signing_marker(&self, commitment: EpochCommitment) -> Option<u64> {
            self.storage
                .ol_checkpoint()
                .get_checkpoint_signing_entry_blocking(commitment)
                .expect("test: read checkpoint signing marker")
        }

        fn reconcile(&self, first_unaccepted_epoch: Epoch) -> ReconcileStats {
            self.reconcile_with_boundaries(
                first_unaccepted_epoch.checked_sub(1),
                first_unaccepted_epoch,
            )
        }

        fn reconcile_with_boundaries(
            &self,
            last_settled_epoch: Option<Epoch>,
            first_unaccepted_epoch: Epoch,
        ) -> ReconcileStats {
            reconcile_unaccepted_checkpoint_artifacts_from_epoch(
                &self.storage,
                last_settled_epoch,
                first_unaccepted_epoch,
                *TEST_MAGIC_BYTES.as_bytes(),
            )
            .expect("test: reconcile checkpoint artifacts")
        }

        fn has_payload_artifact(&self, queued: &QueuedCheckpoint) -> bool {
            self.storage
                .ol_checkpoint()
                .get_checkpoint_payload_entry_blocking(queued.commitment)
                .expect("test: read checkpoint payload artifact")
                .is_some()
        }

        fn bundle_status(&self, payload_idx: u64) -> L1BundleStatus {
            self.storage
                .l1_writer()
                .get_payload_entry_by_idx_blocking(payload_idx)
                .expect("test: read bundle")
                .expect("test: bundle exists")
                .status
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

    /// A broadcaster status for a transaction buried deep enough to be final.
    fn finalized_tx_status() -> L1TxStatus {
        L1TxStatus::Finalized {
            confirmations: 100,
            block_hash: Buf32::zero(),
            block_height: 1,
        }
    }

    /// A transaction the broadcaster sweep cannot parse as a checkpoint.
    fn opaque_transaction() -> Transaction {
        Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: Vec::new(),
            output: Vec::new(),
        }
    }

    #[test]
    fn failed_retiring_checkpoint_clears_matching_signing_marker() {
        let fixture = Fixture::new();
        let epoch = 2;
        let queued = fixture.queue_checkpoint(
            epoch,
            1,
            L1BundleStatus::Retiring,
            EnvelopeTxs::Opaque(L1TxStatus::Published, L1TxStatus::InvalidInputs),
        );
        fixture.put_epoch_summary(&queued);
        fixture.put_artifacts(&queued, true);
        let failed = PayloadCheckpointRef::Checkpoint {
            epoch,
            id: checkpoint_payload_id(&queued.checkpoint),
        };

        CheckpointFailureCleanup::new(Arc::new(fixture.storage.clone()))
            .handle_failed_checkpoint(failed)
            .expect("clean up failed checkpoint");

        assert_eq!(fixture.signing_marker(queued.commitment), None);
    }

    #[test]
    fn failed_retiring_checkpoint_preserves_newer_candidate_marker() {
        let fixture = Fixture::new();
        let epoch = 2;
        let queued = fixture.queue_checkpoint(
            epoch,
            2,
            L1BundleStatus::Retiring,
            EnvelopeTxs::Opaque(L1TxStatus::Published, L1TxStatus::InvalidInputs),
        );
        fixture.put_epoch_summary(&queued);
        fixture.put_artifacts(&queued, true);
        let mut failed_id = checkpoint_payload_id(&queued.checkpoint);
        failed_id.0[0] ^= 1;
        let failed = PayloadCheckpointRef::Checkpoint {
            epoch,
            id: failed_id,
        };

        CheckpointFailureCleanup::new(Arc::new(fixture.storage.clone()))
            .handle_failed_checkpoint(failed)
            .expect("clean up failed checkpoint");

        assert_eq!(
            fixture.signing_marker(queued.commitment),
            Some(queued.intent_idx)
        );
    }

    #[test]
    fn reconcile_deletes_artifacts_and_cancels_unpublished_queue_entry() {
        let fixture = Fixture::new();
        let epoch = 2;
        let queued = fixture.queue_checkpoint(
            epoch,
            9,
            L1BundleStatus::Unpublished,
            EnvelopeTxs::Opaque(L1TxStatus::Unpublished, L1TxStatus::Unpublished),
        );
        fixture.put_artifacts(&queued, true);

        let stats = fixture.reconcile(epoch);

        assert_eq!(stats.deleted_payloads, 1);
        assert_eq!(
            stats.writer,
            WriterCancelStats {
                abandoned_intents: 1,
                abandoned_bundles: 1,
                invalidated_txs: 1,
                ..WriterCancelStats::default()
            }
        );
        assert!(!fixture.has_payload_artifact(&queued));
        assert_eq!(
            fixture.bundle_status(queued.payload_idx),
            L1BundleStatus::Abandoned
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Abandoned
        );
        assert_eq!(
            fixture.tx_status(queued.commit_txid),
            L1TxStatus::InvalidInputs
        );
        // Killing the reveal too would strand the commit output if the commit escaped
        // through the crash window; an unsent commit leaves it an orphan instead.
        assert_eq!(
            fixture.tx_status(queued.reveal_txid),
            L1TxStatus::Unpublished
        );
    }

    #[test]
    fn reconcile_cancels_below_tip_bundle_without_deleting_settled_artifacts() {
        let fixture = Fixture::new();
        let epoch = 2;
        let queued = fixture.queue_checkpoint(
            epoch,
            16,
            L1BundleStatus::Unpublished,
            EnvelopeTxs::Decodable(L1TxStatus::Unpublished, L1TxStatus::Unpublished),
        );
        fixture.put_artifacts(&queued, false);

        let stats = fixture.reconcile(epoch + 1);

        assert_eq!(
            stats,
            ReconcileStats {
                writer: WriterCancelStats {
                    abandoned_intents: 1,
                    abandoned_bundles: 1,
                    invalidated_txs: 1,
                    ..WriterCancelStats::default()
                },
                ..ReconcileStats::default()
            }
        );
        assert!(fixture.has_payload_artifact(&queued));
        assert_eq!(
            fixture.bundle_status(queued.payload_idx),
            L1BundleStatus::Abandoned
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Abandoned
        );
        assert_eq!(
            fixture.tx_status(queued.commit_txid),
            L1TxStatus::InvalidInputs
        );
        assert_eq!(
            fixture.tx_status(queued.reveal_txid),
            L1TxStatus::Unpublished
        );
    }

    #[test]
    fn reconcile_leaves_verified_but_non_final_checkpoint_in_flight() {
        let fixture = Fixture::new();
        let epoch = 2;
        let queued = fixture.queue_checkpoint(
            epoch,
            17,
            L1BundleStatus::Unpublished,
            EnvelopeTxs::Decodable(L1TxStatus::Unpublished, L1TxStatus::Unpublished),
        );
        fixture.put_artifacts(&queued, true);

        let stats = fixture.reconcile_with_boundaries(Some(epoch - 1), epoch + 1);

        assert_eq!(stats, ReconcileStats::default());
        assert!(fixture.has_payload_artifact(&queued));
        assert_eq!(
            fixture.bundle_status(queued.payload_idx),
            L1BundleStatus::Unpublished
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Bundled(queued.payload_idx)
        );
        for txid in [queued.commit_txid, queued.reveal_txid] {
            assert_eq!(fixture.tx_status(txid), L1TxStatus::Unpublished);
        }
    }

    /// A checkpoint can be final on Bitcoin and still be rejected by ASM, leaving its
    /// epoch unaccepted with a bundle the watcher no longer tracks. Reconciliation
    /// deletes the artifacts so the epoch is rebuilt, so it also has to free the intent:
    /// a deterministic rebuild reproduces the same intent commitment, and a still-bundled
    /// intent would deduplicate the retry away and stall the epoch for good.
    #[test]
    fn reconcile_frees_intent_of_finalized_unaccepted_bundle() {
        let fixture = Fixture::new();
        let epoch = 3;
        let queued = fixture.queue_checkpoint(
            epoch,
            21,
            L1BundleStatus::Finalized,
            EnvelopeTxs::Decodable(finalized_tx_status(), finalized_tx_status()),
        );
        fixture.put_artifacts(&queued, true);

        let stats = fixture.reconcile(epoch);

        assert_eq!(stats.deleted_payloads, 1);
        assert_eq!(
            stats.writer,
            WriterCancelStats {
                abandoned_intents: 1,
                ..WriterCancelStats::default()
            }
        );
        // The bundle stays finalized: it is on L1 and cannot be recalled.
        assert_eq!(
            fixture.bundle_status(queued.payload_idx),
            L1BundleStatus::Finalized
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Abandoned
        );
        assert!(!fixture.has_payload_artifact(&queued));
        assert_eq!(fixture.signing_marker(queued.commitment), None);
    }

    /// The settled side must not free the intent: those epochs are already accepted and
    /// nothing rebuilds them, so an abandoned intent would only invite a duplicate.
    #[test]
    fn reconcile_keeps_intent_of_finalized_settled_bundle() {
        let fixture = Fixture::new();
        let epoch = 3;
        let queued = fixture.queue_checkpoint(
            epoch,
            22,
            L1BundleStatus::Finalized,
            EnvelopeTxs::Decodable(finalized_tx_status(), finalized_tx_status()),
        );
        fixture.put_artifacts(&queued, true);

        let stats = fixture.reconcile_with_boundaries(Some(epoch), epoch + 1);

        assert_eq!(stats, ReconcileStats::default());
        assert_eq!(
            fixture.bundle_status(queued.payload_idx),
            L1BundleStatus::Finalized
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Bundled(queued.payload_idx)
        );
        assert!(fixture.has_payload_artifact(&queued));
    }

    #[test]
    fn reconcile_deletes_artifacts_and_retires_escaped_bundle() {
        let fixture = Fixture::new();
        let epoch = 3;
        let queued = fixture.queue_checkpoint(
            epoch,
            11,
            L1BundleStatus::Unpublished,
            EnvelopeTxs::Opaque(L1TxStatus::Published, L1TxStatus::Published),
        );
        fixture.put_artifacts(&queued, false);

        let stats = fixture.reconcile(epoch);

        assert_eq!(stats.deleted_payloads, 1);
        assert_eq!(
            stats.writer,
            WriterCancelStats {
                left_published_bundles: 1,
                ..WriterCancelStats::default()
            }
        );
        assert!(!fixture.has_payload_artifact(&queued));
        assert_eq!(
            fixture.bundle_status(queued.payload_idx),
            L1BundleStatus::Retiring
        );
        assert_eq!(
            fixture.intent_status(queued.intent_idx),
            IntentStatus::Bundled(queued.payload_idx)
        );
        for txid in [queued.commit_txid, queued.reveal_txid] {
            assert_eq!(fixture.tx_status(txid), L1TxStatus::Published);
        }
    }
}
