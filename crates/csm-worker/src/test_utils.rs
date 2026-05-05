//! Shared test helpers for the CSM worker.

use std::sync::Arc;

use bitcoin::Block;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput};
use strata_primitives::{
    epoch::EpochCommitment,
    l1::{L1BlockCommitment, L1BlockId},
};
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

use crate::context::CsmWorkerContext;

/// Test context backed by a real `NodeStorage` and `StatusChannel`.
///
/// Tests that don't exercise the L1 fetch path get a panicking `get_l1_block`.
pub(crate) struct StubCtx {
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    finality_depth: u32,
}

impl StubCtx {
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
        finality_depth: u32,
    ) -> Self {
        Self {
            storage,
            status_channel,
            finality_depth,
        }
    }
}

impl CsmWorkerContext for StubCtx {
    fn put_client_state_update(
        &self,
        block: &L1BlockCommitment,
        output: ClientUpdateOutput,
    ) -> anyhow::Result<()> {
        self.storage
            .client_state()
            .put_update_blocking(block, output)?;
        Ok(())
    }

    fn publish_client_state(&self, state: ClientState, block: L1BlockCommitment) {
        self.status_channel.update_client_state(state, block);
    }

    fn put_checkpoint_l1_ref(
        &self,
        commitment: EpochCommitment,
        observation: CheckpointL1Ref,
    ) -> anyhow::Result<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_l1_ref_blocking(commitment, observation)?;
        Ok(())
    }

    fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<CheckpointPayload>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_checkpoint_payload_entry_blocking(commitment)?)
    }

    fn put_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
    ) -> anyhow::Result<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_payload_entry_blocking(commitment, payload)?;
        Ok(())
    }

    fn get_l1_block(&self, _blockid: &L1BlockId) -> anyhow::Result<Block> {
        panic!("test should not fetch L1 block")
    }

    fn l1_reorg_safe_depth(&self) -> u32 {
        self.finality_depth
    }
}
