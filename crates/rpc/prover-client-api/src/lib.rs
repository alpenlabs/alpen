//! Provides prover-client related APIs for the RPC server.

use std::collections::HashMap;

use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use strata_primitives::{
    evm_exec::EvmEeBlockCommitment, l1::L1BlockCommitment, l2::L2BlockCommitment,
};
use strata_rpc_types::ProofKey;
use zkaleido::ProofReceipt;

/// RPCs related to information about the client itself.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "dev_strata"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "dev_strata"))]
pub trait StrataProverClientApi {
    /// Start proving the given el block
    #[method(name = "proveElBlocks")]
    async fn prove_el_blocks(
        &self,
        el_block_range: (EvmEeBlockCommitment, EvmEeBlockCommitment),
    ) -> RpcResult<Vec<ProofKey>>;

    /// Start proving the given cl block
    #[method(name = "proveOLBlocks")]
    async fn prove_ol_blocks(
        &self,
        ol_block_range: (L2BlockCommitment, L2BlockCommitment),
    ) -> RpcResult<Vec<ProofKey>>;

    /// Start proving the given checkpoint info
    #[method(name = "proveCheckpointRaw")]
    async fn prove_checkpoint_raw(
        &self,
        checkpoint_idx: u64,
        l1_range: (L1BlockCommitment, L1BlockCommitment),
        l2_range: (L2BlockCommitment, L2BlockCommitment),
    ) -> RpcResult<Vec<ProofKey>>;

    /// Start proving the given checkpoint
    #[method(name = "proveCheckpoint")]
    async fn prove_checkpoint(&self, ckp_idx: u64) -> RpcResult<Vec<ProofKey>>;

    /// Start proving the latest checkpoint info from the sequencer
    #[method(name = "proveLatestCheckPoint")]
    async fn prove_latest_checkpoint(&self) -> RpcResult<Vec<ProofKey>>;

    /// Get the task status of `key`
    #[method(name = "getTaskStatus")]
    async fn get_task_status(&self, key: ProofKey) -> RpcResult<String>;

    /// Get proof with the given `key`
    #[method(name = "getProof")]
    async fn get_proof(&self, key: ProofKey) -> RpcResult<Option<ProofReceipt>>;

    /// Get report of the current prover-client
    #[method(name = "getReport")]
    async fn get_report(&self) -> RpcResult<HashMap<String, usize>>;
}
