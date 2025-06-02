//! Strata custom reth rpc

pub mod eth;
mod rpc;
pub mod sequencer;

use alpen_reth_statediff::BlockStateDiff;
pub use eth::{StrataEthApi, StrataNodeCore};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use revm_primitives::alloy_primitives::B256;
pub use rpc::StrataRPC;
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

    /// Returns the state diff for the block.
    #[method(name = "getBlockStateDiff")]
    fn get_block_state_diff(&self, block_hash: B256) -> RpcResult<Option<BlockStateDiff>>;

    /// Returns the state root for the block_number as reconstructured from the state diffs.
    #[method(name = "getStateRootByDiffs")]
    fn get_state_root_via_diffs(&self, block_number: u64) -> RpcResult<Option<B256>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[expect(clippy::large_enum_variant)]
pub enum BlockWitness {
    Raw(#[serde(with = "hex::serde")] Vec<u8>),
    Json(EvmBlockStfInput),
}
