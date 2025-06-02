use alloy_rpc_types::{Block, Header};
use jsonrpsee::{core::client::ClientT, http_client::HttpClient, rpc_params};
use strata_primitives::{
    buf::Buf32,
    evm_exec::EvmEeBlockCommitment,
    proof::{ProofContext, ProofKey},
};
use strata_proofimpl_evm_ee_stf::{
    primitives::EvmEeProofInput, program::EvmEeProgram, EvmBlockStfInput,
};
use strata_rocksdb::prover::db::ProofDb;
use tracing::error;

use super::ProvingOp;
use crate::errors::ProvingTaskError;

/// A struct that implements the [`ProvingOp`] trait for EVM Execution Environment (EE) State
/// Transition Function (STF) proofs.
///
/// It is responsible for interfacing with the `Reth` client and fetching necessary data required by
/// the [`EvmEeProgram`] for the proof generation.
#[derive(Debug, Clone)]
pub(crate) struct EvmEeOperator {
    el_client: HttpClient,
}

impl EvmEeOperator {
    /// Creates a new EL operations instance.
    pub(crate) fn new(el_client: HttpClient) -> Self {
        Self { el_client }
    }

    /// Retrieves the EVM EE [`Block`] for a given block number.
    pub(crate) async fn get_block(&self, block_num: u64) -> Result<Block, ProvingTaskError> {
        self.el_client
            .request(
                "eth_getBlockByNumber",
                rpc_params![format!("0x{:x}", block_num), false],
            )
            .await
            .inspect_err(|_| error!(%block_num, "Failed to fetch EVM Block"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))
    }

    /// Retrieves the EVM EE [`Block`] for a given block number.
    async fn get_block_header(&self, blkid: Buf32) -> Result<Header, ProvingTaskError> {
        let block: Block = self
            .el_client
            .request("eth_getBlockByHash", rpc_params![blkid, false])
            .await
            .inspect_err(|_| error!(%blkid, "Failed to fetch EVM Block Header"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;
        Ok(block.header)
    }
}

impl ProvingOp for EvmEeOperator {
    type Program = EvmEeProgram;
    type Params = (EvmEeBlockCommitment, EvmEeBlockCommitment);

    fn construct_proof_ctx(
        &self,
        block_range: &Self::Params,
    ) -> Result<ProofContext, ProvingTaskError> {
        let (start_blk, end_blk) = *block_range;
        Ok(ProofContext::EvmEeStf(start_blk, end_blk))
    }

    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        _db: &ProofDb,
    ) -> Result<EvmEeProofInput, ProvingTaskError> {
        let (start_block, end_block) = match task_id.context() {
            ProofContext::EvmEeStf(start, end) => (*start, *end),
            _ => return Err(ProvingTaskError::InvalidInput("EvmEe".to_string())),
        };

        let mut mini_batch = Vec::new();

        let mut blkid = *end_block.blkid();
        loop {
            let witness: EvmBlockStfInput = self
                .el_client
                .request("strataee_getBlockWitness", rpc_params![blkid, true])
                .await
                .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;

            mini_batch.push(witness);

            if start_block.blkid() == &blkid {
                break;
            } else {
                blkid = Buf32(
                    self.get_block_header(blkid.as_ref().into())
                        .await
                        .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?
                        .parent_hash
                        .into(),
                );
            }
        }
        mini_batch.reverse();

        Ok(mini_batch)
    }
}
