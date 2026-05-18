//! Shared test helpers for the CSM worker.

use std::{collections::HashMap, sync::Arc};

use bitcoin::Block;
use strata_asm_common::AuxData;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput};
use strata_identifiers::Epoch;
use strata_l1_txfmt::MagicBytes;
use strata_primitives::{
    L1Height,
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
    genesis_l1_block: L1BlockCommitment,
    l1_fetch: L1Fetch,
    /// Canonical ASM states keyed by L1 height, used to serve gap-fill walks.
    canonical_asm_states: HashMap<L1Height, (L1BlockId, AsmState)>,
    /// Height at which `get_canonical_l1_block` should fail, simulating a gap
    /// block that can't be resolved.
    canonical_fail_height: Option<L1Height>,
    /// When set, `put_client_state_update` fails, simulating a commit failure
    /// after a block's logs were processed.
    fail_client_state_update: bool,
}

impl StubCtx {
    pub(crate) fn new(
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
        finality_depth: u32,
        magic: MagicBytes,
        genesis_l1_block: L1BlockCommitment,
    ) -> Self {
        Self {
            storage,
            status_channel,
            finality_depth,
            magic,
            genesis_l1_block,
            l1_fetch: L1Fetch::Unset,
            canonical_asm_states: HashMap::new(),
            canonical_fail_height: None,
            fail_client_state_update: false,
        }
    }

    /// Configures `get_l1_block` to return an error on any blockid.
    pub(crate) fn with_l1_fetch_failure(mut self) -> Self {
        self.l1_fetch = L1Fetch::Fail;
        self
    }

    /// Registers a canonical ASM state at `height` so gap-fill can walk it.
    pub(crate) fn with_canonical_asm_state(
        mut self,
        height: L1Height,
        blkid: L1BlockId,
        state: AsmState,
    ) -> Self {
        self.canonical_asm_states.insert(height, (blkid, state));
        self
    }

    /// Makes `get_canonical_l1_block` fail at `height`, simulating an
    /// unresolvable gap block.
    pub(crate) fn with_canonical_failure_at(mut self, height: L1Height) -> Self {
        self.canonical_fail_height = Some(height);
        self
    }

    /// Makes `put_client_state_update` fail, simulating a commit failure after
    /// a block's logs were processed.
    pub(crate) fn with_commit_failure(mut self) -> Self {
        self.fail_client_state_update = true;
        self
    }
}

impl CsmWorkerContext for StubCtx {
    fn put_client_state_update(
        &self,
        block: &L1BlockCommitment,
        output: ClientUpdateOutput,
    ) -> anyhow::Result<()> {
        if self.fail_client_state_update {
            return Err(anyhow::anyhow!("simulated client state update failure"));
        }
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
        self.canonical_asm_states
            .get(&block.height())
            .filter(|(blkid, _)| blkid == block.blkid())
            .map(|(_, state)| state.clone())
            .ok_or_else(|| anyhow::anyhow!("no test ASM state configured for {block}"))
    }

    fn get_aux_data(&self, block: &L1BlockCommitment) -> anyhow::Result<AuxData> {
        Err(anyhow::anyhow!(
            "stub get_aux_data called for {block}; tests that need aux data should use end-to-end fixtures"
        ))
    }

    fn get_canonical_l1_block(&self, height: L1Height) -> anyhow::Result<L1BlockCommitment> {
        if self.canonical_fail_height == Some(height) {
            return Err(anyhow::anyhow!(
                "simulated canonical lookup failure at height {height}"
            ));
        }
        self.canonical_asm_states
            .get(&height)
            .map(|(blkid, _)| L1BlockCommitment::new(height, *blkid))
            .ok_or_else(|| anyhow::anyhow!("no test canonical block configured at height {height}"))
    }

    fn fetch_most_recent_client_state(
        &self,
    ) -> anyhow::Result<Option<(L1BlockCommitment, ClientState)>> {
        Ok(self.storage.client_state().fetch_most_recent_state()?)
    }

    fn genesis_l1_block(&self) -> L1BlockCommitment {
        self.genesis_l1_block
    }

    fn get_last_checkpoint_l1_ref_epoch(&self) -> anyhow::Result<Option<EpochCommitment>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_last_checkpoint_l1_ref_epoch_blocking()?)
    }

    fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> anyhow::Result<Option<EpochCommitment>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_canonical_epoch_commitment_at_blocking(epoch)?)
    }

    fn get_checkpoint_l1_ref(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<CheckpointL1Ref>> {
        Ok(self
            .storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_blocking(commitment)?)
    }
}
