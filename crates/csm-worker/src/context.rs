//! External operations the CSM worker performs during state transitions.

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

use crate::errors::CsmWorkerResult;

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
    ) -> CsmWorkerResult<()>;

    /// Publishes the current client state and the L1 block it is anchored at.
    fn publish_client_state(&self, state: ClientState, block: L1BlockCommitment);

    /// Atomically records the L1-observed checkpoint payload and L1 ref for
    /// `commitment`.
    fn put_checkpoint_l1_observation(
        &self,
        commitment: EpochCommitment,
        payload: CheckpointPayload,
        l1_ref: CheckpointL1Ref,
    ) -> CsmWorkerResult<()>;

    /// Fetches an L1 block by its block id.
    fn get_l1_block(&self, blockid: &L1BlockId) -> CsmWorkerResult<Block>;

    /// L1 reorg-safe depth used to decide checkpoint finality.
    fn l1_reorg_safe_depth(&self) -> u32;

    /// SPS-50 magic bytes used to identify protocol transactions.
    fn magic_bytes(&self) -> MagicBytes;

    /// Fetches the ASM state recorded at `block`.
    fn get_asm_state(&self, block: &L1BlockCommitment) -> CsmWorkerResult<AsmState>;

    /// Fetches the auxiliary data ASM consumed when processing `block`.
    fn get_aux_data(&self, block: &L1BlockCommitment) -> CsmWorkerResult<AuxData>;

    /// Resolves the canonical L1 block commitment at `height`.
    fn get_canonical_l1_block(&self, height: L1Height) -> CsmWorkerResult<L1BlockCommitment>;

    /// Returns the most recently persisted client state, or `None` if storage
    /// has none yet.
    fn fetch_most_recent_client_state(
        &self,
    ) -> CsmWorkerResult<Option<(L1BlockCommitment, ClientState)>>;

    /// L1 block that bootstrap should anchor to when storage has no client
    /// state yet.
    fn genesis_l1_block(&self) -> L1BlockCommitment;

    /// Returns the epoch of the most recent L1-observed checkpoint, or `None`
    /// if nothing has been observed yet.
    fn get_last_checkpoint_l1_ref_epoch(&self) -> CsmWorkerResult<Option<EpochCommitment>>;

    /// Returns the canonical epoch commitment at `epoch`, if recorded.
    fn get_canonical_epoch_commitment_at(
        &self,
        epoch: Epoch,
    ) -> CsmWorkerResult<Option<EpochCommitment>>;

    /// Returns the recorded L1 ref for an observed checkpoint at `commitment`.
    fn get_checkpoint_l1_ref(
        &self,
        commitment: EpochCommitment,
    ) -> CsmWorkerResult<Option<CheckpointL1Ref>>;

    /// Returns the L1-observed checkpoint payload at `commitment` (carries the
    /// tip the checkpoint declared). Paired with [`Self::get_checkpoint_l1_ref`]
    /// to reconstruct the full observation record at bootstrap.
    fn get_checkpoint_payload(
        &self,
        commitment: EpochCommitment,
    ) -> CsmWorkerResult<Option<CheckpointPayload>>;
}
