//! Strata custom reth rpc

pub mod eth;
mod rpc;
pub mod sequencer;

use alpen_reth_statediff::BatchStateDiffSerde;
pub use eth::{AlpenEthApi, StrataNodeCore};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use revm_primitives::alloy_primitives::B256;
pub use rpc::AlpenRPC;
pub use sequencer::SequencerClient;
use serde::{Deserialize, Serialize};
use strata_proofimpl_evm_ee_stf::EvmBlockStfInput;

#[cfg_attr(not(test), rpc(server, namespace = "strataee"))]
#[cfg_attr(test, rpc(server, client, namespace = "strataee"))]
pub trait StrataRpcApi {
    /// Returns the state changesets with storage proofs for requested blocks.
    /// Used as part of input to riscvm during proof generation
    #[method(name = "getBlockWitness")]
    fn get_block_witness(
        &self,
        block_hash: B256,
        json: Option<bool>,
    ) -> RpcResult<Option<BlockWitness>>;

    /// Returns the state diff for a single block.
    ///
    /// N.B. Implemented for testing primarily, should not be used in production API.
    #[method(name = "getStateDiffForBlock")]
    fn get_state_diff_for_block(&self, block_hash: B256) -> RpcResult<Option<BatchStateDiffSerde>>;

    /// Returns the state root for the block_number as reconstructured from the state diffs.
    ///
    /// N.B. Implemented for testing primarily, should not be used in production API.
    /// The genesis state is hardcoded to be taken from dev config.
    #[method(name = "getStateRootByDiffs")]
    fn get_state_root_via_diffs(&self, block_number: u64) -> RpcResult<Option<B256>>;

    /// Returns the aggregated state diff for a range of blocks.
    ///
    /// N.B. Implemented for testing primarily, should not be used in production API.
    #[method(name = "getStateDiffForRange")]
    fn get_state_diff_for_range(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> RpcResult<Option<BatchStateDiffSerde>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[expect(clippy::large_enum_variant, reason = "I don't want to box it")]
pub enum BlockWitness {
    Raw(#[serde(with = "hex::serde")] Vec<u8>),
    Json(EvmBlockStfInput),
}
