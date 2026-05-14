//! External operations the CSM worker performs during state transitions.

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

    /// Atomically records the L1-observed checkpoint payload and L1 ref for
    /// `commitment`.
    fn put_checkpoint_l1_observation(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
        l1_ref: CheckpointL1Ref,
    ) -> anyhow::Result<()>;

    /// Fetches an L1 block by its block id.
    fn get_l1_block(&self, blockid: &L1BlockId) -> anyhow::Result<Block>;

    /// L1 reorg-safe depth used to decide checkpoint finality.
    fn l1_reorg_safe_depth(&self) -> u32;

    /// SPS-50 magic bytes used to identify protocol transactions.
    fn magic_bytes(&self) -> MagicBytes;

    /// Fetches the ASM state recorded at `block`.
    fn get_asm_state(&self, block: &L1BlockCommitment) -> anyhow::Result<AsmState>;

    /// Fetches the auxiliary data ASM consumed when processing `block`.
    fn get_aux_data(&self, block: &L1BlockCommitment) -> anyhow::Result<AuxData>;
}
