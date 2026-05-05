//! External operations the CSM worker performs during state transitions.

use bitcoin::Block;
use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput};
use strata_primitives::{
    epoch::EpochCommitment,
    l1::{L1BlockCommitment, L1BlockId},
};

/// Operations the worker delegates to the outside world: persistence, status
/// publishing, and L1 fetch.
///
/// Kept as a trait so tests can swap in a stub without spinning up real storage
/// or a Bitcoin RPC client.
pub trait CsmWorkerContext: Send + Sync {
    /// Writes a client state update for the given L1 block.
    fn put_client_state_update(
        &self,
        block: &L1BlockCommitment,
        output: ClientUpdateOutput,
    ) -> anyhow::Result<()>;

    /// Publishes the current client state and the L1 block it is anchored at.
    fn publish_client_state(&self, state: ClientState, block: L1BlockCommitment);

    /// Records that the given checkpoint epoch was observed on L1.
    fn put_checkpoint_l1_ref(
        &self,
        commitment: EpochCommitment,
        observation: CheckpointL1Ref,
    ) -> anyhow::Result<()>;

    /// Returns the persisted checkpoint payload for `commitment`, if any.
    fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> anyhow::Result<Option<CheckpointPayload>>;

    /// Persists a checkpoint payload extracted from L1.
    fn put_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
    ) -> anyhow::Result<()>;

    /// Fetches an L1 block by its block id.
    fn get_l1_block(&self, blockid: &L1BlockId) -> anyhow::Result<Block>;

    /// L1 reorg-safe depth used to decide checkpoint finality.
    fn l1_reorg_safe_depth(&self) -> u32;
}
