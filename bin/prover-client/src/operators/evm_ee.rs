use alloy_rpc_types::{Block, Header};
use jsonrpsee::{core::client::ClientT, http_client::HttpClient, rpc_params};
use strata_db_store_sled::prover::ProofDBSled;
use strata_primitives::{buf::Buf32, proof::ProofKey};
use strata_proofimpl_evm_ee_stf::{primitives::EvmEeProofInput, EvmBlockStfInput};
use tracing::error;

use super::ProofInputFetcher;
use crate::errors::ProvingTaskError;

/// Operator for EVM Execution Environment (EE) State Transition Function (STF) proof generation.
///
/// Provides access to EL client for fetching data needed for EVM EE STF proofs.
#[derive(Debug, Clone)]
pub(crate) struct EvmEeOperator {
    el_client: HttpClient,
}

impl EvmEeOperator {
    /// Creates a new EVM EE operator.
    pub(crate) fn new(el_client: HttpClient) -> Self {
        Self { el_client }
    }

    /// Fetches EVM EE block header by block hash.
    async fn get_block_header(&self, blkid: Buf32) -> Result<Header, ProvingTaskError> {
        let block: Block = self
            .el_client
            .request("eth_getBlockByHash", rpc_params![blkid, false])
            .await
            .inspect_err(|_| error!(%blkid, "Failed to fetch EVM Block"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;
        Ok(block.header)
    }

    /// Fetches EVM EE block header by block height.
    pub(crate) async fn get_block_header_by_height(
        &self,
        block_num: u64,
    ) -> Result<Header, ProvingTaskError> {
        let block: Block = self
            .el_client
            .request(
                "eth_getBlockByNumber",
                rpc_params![format!("0x{:x}", block_num), false],
            )
            .await
            .inspect_err(|_| error!(%block_num, "Failed to fetch EVM Block"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;
        Ok(block.header)
    }
}

impl ProofInputFetcher for EvmEeOperator {
    type Input = EvmEeProofInput;

    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        _db: &ProofDBSled,
    ) -> Result<Self::Input, ProvingTaskError> {
        let (start_block, end_block) = match task_id.context() {
            strata_primitives::proof::ProofContext::EvmEeStf(start, end) => (*start, *end),
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
