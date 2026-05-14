//! Shared test helpers for the CSM worker.

use std::sync::Arc;

use bitcoin::Block;
use strata_asm_common::AuxData;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput};
use strata_l1_txfmt::MagicBytes;
use strata_primitives::{
    epoch::EpochCommitment,
    l1::{L1BlockCommitment, L1BlockId},
};
use strata_state::asm_state::AsmState;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

use crate::context::CsmWorkerContext;

/// What `get_l1_block` does when called.
enum L1Fetch {
    /// Caller didn't configure a fetch result; panic if requested.
    Unset,
    /// Return an error on any blockid.
    Fail,
}

/// Test context backed by a real `NodeStorage` and `StatusChannel`.
pub(crate) struct StubCtx {
    storage: Arc<NodeStorage>,
    status_channel: Arc<StatusChannel>,
    finality_depth: u32,
    magic: MagicBytes,
    l1_fetch: L1Fetch,
}

impl StubCtx {
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
        finality_depth: u32,
        magic: MagicBytes,
    ) -> Self {
        Self {
            storage,
            status_channel,
            finality_depth,
            magic,
            l1_fetch: L1Fetch::Unset,
        }
    }

    /// Configures `get_l1_block` to return an error on any blockid.
    pub(crate) fn with_l1_fetch_failure(mut self) -> Self {
        self.l1_fetch = L1Fetch::Fail;
        self
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

    fn put_checkpoint_l1_observation(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
        l1_ref: CheckpointL1Ref,
    ) -> anyhow::Result<()> {
        self.storage
            .ol_checkpoint()
            .put_checkpoint_l1_observation_blocking(commitment, payload, l1_ref)?;
        Ok(())
    }

    fn get_l1_block(&self, _blockid: &L1BlockId) -> anyhow::Result<Block> {
        match &self.l1_fetch {
            L1Fetch::Unset => panic!("test should not fetch L1 block"),
            L1Fetch::Fail => Err(anyhow::anyhow!("simulated L1 fetch failure")),
        }
    }

    fn l1_reorg_safe_depth(&self) -> u32 {
        self.finality_depth
    }

    fn magic_bytes(&self) -> MagicBytes {
        self.magic
    }

    fn get_asm_state(&self, block: &L1BlockCommitment) -> anyhow::Result<AsmState> {
        Err(anyhow::anyhow!(
            "stub get_asm_state called for {block}; tests that need ASM state should use end-to-end fixtures"
        ))
    }

    fn get_aux_data(&self, block: &L1BlockCommitment) -> anyhow::Result<AuxData> {
        Err(anyhow::anyhow!(
            "stub get_aux_data called for {block}; tests that need aux data should use end-to-end fixtures"
        ))
    }
}
