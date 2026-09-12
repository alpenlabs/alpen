//! Node storage adapter for checkpoint publication and recovery.

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use strata_asm_common::{SectionStateExt, Subprotocol};
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
use strata_checkpoint_types::CheckpointProofTask;
use strata_db_types::{
    backend::DatabaseBackend,
    l1_broadcast::{L1BroadcastDatabase, L1TxEntry},
    l1_writer::{BundledPayloadEntry, IntentEntry, L1WriterDatabase},
    ol_checkpoint::OLCheckpointDatabase,
};
use strata_identifiers::{Buf32, Epoch, EpochCommitment};
use strata_ol_sequencer::{
    CheckpointContextError, CheckpointContextResult, CheckpointPublishContext,
    CheckpointReconcileContext,
};
use strata_storage::NodeStorage;
use tracing::debug;

/// Adapts node storage to the capabilities used by checkpoint L1 lifecycle logic.
pub(crate) struct NodeCheckpointContext {
    storage: Arc<NodeStorage>,
}

impl NodeCheckpointContext {
    pub(crate) fn new(storage: Arc<NodeStorage>) -> Self {
        Self { storage }
    }
}

#[async_trait]
impl CheckpointPublishContext for NodeCheckpointContext {
    async fn get_accepted_checkpoint_epoch(&self) -> CheckpointContextResult<Option<Epoch>> {
        let Some((_, state)) = self.storage.fetch_canonical_asm_state_async().await? else {
            return Ok(None);
        };
        Ok(state
            .state()
            .find_section(<CheckpointSubprotocol as Subprotocol>::ID)
            .and_then(|section| section.try_to_state::<CheckpointSubprotocol>().ok())
            .map(|state| state.verified_tip().epoch))
    }

    async fn get_safe_checkpoint_epoch(&self) -> CheckpointContextResult<Option<Epoch>> {
        let Some((_, state)) = self.storage.fetch_canonical_client_state_async().await? else {
            return Err(CheckpointContextError::CanonicalStateUnavailable { state: "client" });
        };
        Ok(state
            .get_declared_final_epoch()
            .map(|commitment| commitment.epoch()))
    }

    fn get_next_broadcast_idx(&self) -> CheckpointContextResult<u64> {
        Ok(self.storage.db().broadcast_db().get_next_tx_idx()?)
    }

    fn get_broadcast_entry(&self, idx: u64) -> CheckpointContextResult<Option<L1TxEntry>> {
        Ok(self.storage.db().broadcast_db().get_tx_entry(idx)?)
    }

    fn get_broadcast_entry_by_id(&self, txid: Buf32) -> CheckpointContextResult<Option<L1TxEntry>> {
        Ok(self.storage.db().broadcast_db().get_tx_entry_by_id(txid)?)
    }
}

impl CheckpointReconcileContext for NodeCheckpointContext {
    fn get_first_unaccepted_checkpoint_epoch(&self) -> CheckpointContextResult<Option<Epoch>> {
        let Some((asm_l1, asm_state)) = self
            .storage
            .fetch_canonical_asm_state_blocking()
            .context("fetch canonical ASM state")?
        else {
            debug!(
                "canonical ASM state is not available; skipping checkpoint artifact reconciliation"
            );
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

    fn get_checkpoint_payload_commitments_from_epoch(
        &self,
        epoch: Epoch,
    ) -> CheckpointContextResult<Vec<EpochCommitment>> {
        Ok(self
            .storage
            .db()
            .ol_checkpoint_db()
            .get_checkpoint_payload_commitments_from_epoch(epoch)?)
    }

    fn get_last_summarized_epoch(&self) -> CheckpointContextResult<Option<Epoch>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_last_summarized_epoch_blocking()?)
    }

    fn get_epoch_commitments_at(
        &self,
        epoch: Epoch,
    ) -> CheckpointContextResult<Vec<EpochCommitment>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_epoch_commitments_at_blocking(epoch)?)
    }

    fn get_next_intent_idx(&self) -> CheckpointContextResult<u64> {
        Ok(self.storage.db().writer_db().get_next_intent_idx()?)
    }

    fn get_intent_by_idx(&self, idx: u64) -> CheckpointContextResult<Option<IntentEntry>> {
        Ok(self.storage.db().writer_db().get_intent_by_idx(idx)?)
    }

    fn abandon_unbundled_intent(
        &self,
        intent: IntentEntry,
        payload: BundledPayloadEntry,
    ) -> CheckpointContextResult<()> {
        let commitment = *intent.intent.commitment();
        self.storage
            .db()
            .writer_db()
            .bundle_intent_payload(commitment, intent, payload)?;
        Ok(())
    }

    fn get_payload_by_idx(&self, idx: u64) -> CheckpointContextResult<Option<BundledPayloadEntry>> {
        Ok(self
            .storage
            .db()
            .writer_db()
            .get_payload_entry_by_idx(idx)?)
    }

    fn put_payload(&self, idx: u64, payload: BundledPayloadEntry) -> CheckpointContextResult<()> {
        Ok(self
            .storage
            .db()
            .writer_db()
            .put_payload_entry(idx, payload)?)
    }

    fn get_broadcast_entry_by_id(&self, txid: Buf32) -> CheckpointContextResult<Option<L1TxEntry>> {
        Ok(self.storage.db().broadcast_db().get_tx_entry_by_id(txid)?)
    }

    fn put_broadcast_entry(&self, txid: Buf32, entry: L1TxEntry) -> CheckpointContextResult<()> {
        self.storage.db().broadcast_db().put_tx_entry(txid, entry)?;
        Ok(())
    }

    fn delete_checkpoint_proof(
        &self,
        commitment: EpochCommitment,
    ) -> CheckpointContextResult<bool> {
        Ok(self.storage.checkpoint_proof().del_proof(commitment)?)
    }

    fn delete_checkpoint_prover_task(
        &self,
        commitment: EpochCommitment,
    ) -> CheckpointContextResult<bool> {
        let task_key = CheckpointProofTask(commitment).to_key_bytes();
        Ok(self.storage.prover_tasks().delete_task(&task_key)?)
    }

    fn delete_unobserved_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> CheckpointContextResult<bool> {
        Ok(self
            .storage
            .db()
            .ol_checkpoint_db()
            .del_local_checkpoint_payload_if_unobserved(commitment)?)
    }
}
