use std::sync::Arc;

use jsonrpsee::http_client::HttpClient;
use strata_db_store_sled::prover::ProofDBSled;
use strata_db_types::traits::ProofDatabase;
use strata_ol_chain_types::{L2Block, L2BlockId, L2Header};
use strata_ol_chainstate_types::Chainstate;
use strata_params::RollupParams;
use strata_primitives::{
    buf::Buf32,
    evm_exec::EvmEeBlockCommitment,
    l2::L2BlockCommitment,
    proof::{ProofContext, ProofKey},
};
use strata_proofimpl_ol_stf::program::OLStfInput;
use strata_rpc_api::StrataApiClient;
use strata_rpc_types::RpcBlockHeader;
use strata_zkvm_hosts::get_verification_key;
use tracing::{error, info};

use super::{evm_ee::EvmEeOperator, ProofInputFetcher};
use crate::errors::ProvingTaskError;

/// Operator for Orchestration Layer (OL) State Transition Function (STF) proof generation.
///
/// Provides access to OL client and methods for fetching data needed for OL STF proofs.
#[derive(Debug, Clone)]
pub(crate) struct OLStfOperator {
    pub ol_client: HttpClient,
    evm_ee_operator: Arc<EvmEeOperator>,
    rollup_params: Arc<RollupParams>,
}

impl OLStfOperator {
    /// Creates a new OL STF operator.
    pub(crate) fn new(
        ol_client: HttpClient,
        evm_ee_operator: Arc<EvmEeOperator>,
        rollup_params: Arc<RollupParams>,
    ) -> Self {
        Self {
            ol_client,
            evm_ee_operator,
            rollup_params,
        }
    }

    /// Creates and stores the EvmEeStf proof dependencies for a OLStf proof.
    ///
    /// This fetches the L2 blocks in the range to get their exec commitments and creates
    /// EvmEeStf proof contexts. Returns the EvmEeStf contexts that need to be submitted.
    pub(crate) async fn create_ol_stf_deps(
        &self,
        start_block: L2BlockCommitment,
        end_block: L2BlockCommitment,
        db: &ProofDBSled,
    ) -> Result<Vec<ProofContext>, ProvingTaskError> {
        info!(
            ?start_block,
            ?end_block,
            "Creating EvmEeStf dependencies for OL Stf"
        );

        // Check if dependencies already exist
        let ol_stf_ctx = ProofContext::OLStf(start_block, end_block);
        if let Some(existing_deps) = db
            .get_proof_deps(ol_stf_ctx)
            .map_err(ProvingTaskError::DatabaseError)?
        {
            info!("OL Stf dependencies already exist, skipping creation");
            return Ok(existing_deps);
        }

        // Get exec commitments from the L2 blocks
        let start_exec = self.get_exec_commitment(*start_block.blkid()).await?;
        let end_exec = self.get_exec_commitment(*end_block.blkid()).await?;

        // Create EvmEeStf proof context
        let evm_ee_ctx = ProofContext::EvmEeStf(start_exec, end_exec);

        // Store OL Stf dependencies (EvmEeStf)
        db.put_proof_deps(ol_stf_ctx, vec![evm_ee_ctx])
            .map_err(ProvingTaskError::DatabaseError)?;

        Ok(vec![evm_ee_ctx])
    }

    /// Fetches L2 block header by block ID.
    async fn get_l2_block_header(
        &self,
        blkid: L2BlockId,
    ) -> Result<RpcBlockHeader, ProvingTaskError> {
        let header = self
            .ol_client
            .get_header_by_id(blkid)
            .await
            .inspect_err(|_| error!(%blkid, "Failed to fetch corresponding ee data"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?
            .ok_or_else(|| {
                error!(%blkid, "L2 Block not found");
                ProvingTaskError::InvalidWitness(format!("L2 Block {blkid} not found"))
            })?;

        Ok(header)
    }

    /// Retrieves the EVM EE block commitment corresponding to the given L2 block ID.
    pub(crate) async fn get_exec_commitment(
        &self,
        ol_block_id: L2BlockId,
    ) -> Result<EvmEeBlockCommitment, ProvingTaskError> {
        let header = self.get_l2_block_header(ol_block_id).await?;
        let ee_header = self
            .evm_ee_operator
            .get_block_header_by_height(header.block_idx)
            .await?;

        Ok(EvmEeBlockCommitment::new(
            ee_header.number,
            Buf32(ee_header.hash.into()),
        ))
    }

    /// Retrieves the chainstate before the given block is applied.
    pub(crate) async fn get_chainstate_before(
        &self,
        blkid: L2BlockId,
    ) -> Result<Chainstate, ProvingTaskError> {
        let raw_witness: Vec<u8> = self
            .ol_client
            .get_ol_block_witness_raw(blkid)
            .await
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;
        let (chainstate, _): (Chainstate, L2Block) =
            borsh::from_slice(&raw_witness).expect("invalid witness");
        Ok(chainstate)
    }

    /// Retrieves the L2 block for the given block ID.
    pub(crate) async fn get_block(&self, blkid: &L2BlockId) -> Result<L2Block, ProvingTaskError> {
        let raw_witness: Vec<u8> = self
            .ol_client
            .get_ol_block_witness_raw(*blkid)
            .await
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;
        let (_, blk): (Chainstate, L2Block) =
            borsh::from_slice(&raw_witness).expect("invalid witness");
        Ok(blk)
    }
}

impl ProofInputFetcher for OLStfOperator {
    type Input = OLStfInput;

    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        db: &ProofDBSled,
    ) -> Result<Self::Input, ProvingTaskError> {
        let (start_block, end_block) = match task_id.context() {
            strata_primitives::proof::ProofContext::OLStf(start, end) => (*start, *end),
            _ => return Err(ProvingTaskError::InvalidInput("OL_STF".to_string())),
        };

        let deps = db
            .get_proof_deps(*task_id.context())
            .map_err(ProvingTaskError::DatabaseError)?
            .ok_or(ProvingTaskError::DependencyNotFound(*task_id))?;

        // sanity check
        // OL STF can have at most 2 deps
        // 1. It will always have EVM EE Proof as a dependency
        // 2. If the OL STF includes terminal block, it also has BTC Blockspace Proof as a
        //    dependency
        assert!(deps.len() <= 2, "invalid OL STF deps");

        // First dependency is always EVM EE Proof
        let evm_ee_id = deps.first().ok_or(ProvingTaskError::NoTasksFound)?;
        let evm_ee_key = ProofKey::new(*evm_ee_id, *task_id.host());
        let evm_ee_proof = db
            .get_proof(&evm_ee_key)
            .map_err(ProvingTaskError::DatabaseError)?
            .ok_or(ProvingTaskError::ProofNotFound(evm_ee_key))?;
        let evm_ee_vk = get_verification_key(&evm_ee_key);
        let evm_ee_proof_with_vk = (evm_ee_proof, evm_ee_vk);

        let chainstate = self.get_chainstate_before(*start_block.blkid()).await?;
        let mut l2_blocks = vec![];
        let mut current_block_hash = *end_block.blkid();

        loop {
            let l2_block = self.get_block(&current_block_hash).await?;
            let prev_l2_blkid = *l2_block.header().parent();
            l2_blocks.push(l2_block);

            if start_block.blkid() == &current_block_hash {
                break;
            } else {
                current_block_hash = prev_l2_blkid;
            }
        }
        l2_blocks.reverse();

        let parent_header = self
            .get_block(&l2_blocks[0].header().get_blockid())
            .await?
            .header()
            .header()
            .clone();

        let rollup_params = self.rollup_params.as_ref().clone();
        Ok(OLStfInput {
            rollup_params,
            parent_header,
            chainstate,
            l2_blocks,
            evm_ee_proof_with_vk,
        })
    }
}
