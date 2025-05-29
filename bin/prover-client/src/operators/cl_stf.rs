use std::sync::Arc;

use jsonrpsee::http_client::HttpClient;
use strata_db::traits::ProofDatabase;
use strata_primitives::{
    buf::Buf32,
    evm_exec::EvmEeBlockCommitment,
    l1::L1BlockCommitment,
    l2::L2BlockCommitment,
    params::RollupParams,
    proof::{ProofContext, ProofKey},
};
use strata_proofimpl_cl_stf::program::{ClStfInput, ClStfProgram};
use strata_rocksdb::prover::db::ProofDb;
use strata_rpc_api::StrataApiClient;
use strata_rpc_types::RpcBlockHeader;
use strata_state::{block::L2Block, chain_state::Chainstate, header::L2Header, id::L2BlockId};
use strata_zkvm_hosts::get_verification_key;
use tokio::sync::Mutex;
use tracing::error;

use super::{
    btc::{BtcBlockscanParams, BtcBlockspaceOperator},
    evm_ee::EvmEeOperator,
    ProvingOp,
};
use crate::{errors::ProvingTaskError, task_tracker::TaskTracker};

/// A struct that implements the [`ProvingOp`] trait for Consensus Layer (CL) State Transition
/// Function (STF) proof generation.
///
/// It is responsible for managing the data and tasks required to generate proofs for CL state
/// transitions. It fetches the necessary inputs for the [`ClStfProgram`] by:
///
/// - Utilizing the [`EvmEeOperator`] to create and manage proving tasks for EVM Execution
///   Environment (EE) STF proofs. The resulting EVM EE STF proof is incorporated as part of the
///   input for the CL STF proof.
/// - Interfacing with the CL Client to fetch additional required information for CL state
///   transition proofs.
#[derive(Debug, Clone)]
pub(crate) struct ClStfOperator {
    pub cl_client: HttpClient,
    evm_ee_operator: Arc<EvmEeOperator>,
    btc_blockspace_operator: Arc<BtcBlockspaceOperator>,
    rollup_params: Arc<RollupParams>,
}

impl ClStfOperator {
    /// Creates a new CL operations instance.
    pub(crate) const fn new(
        cl_client: HttpClient,
        evm_ee_operator: Arc<EvmEeOperator>,
        btc_blockspace_operator: Arc<BtcBlockspaceOperator>,
        rollup_params: Arc<RollupParams>,
    ) -> Self {
        Self {
            cl_client,
            evm_ee_operator,
            btc_blockspace_operator,
            rollup_params,
        }
    }

    async fn get_l2_block_header(
        &self,
        blkid: L2BlockId,
    ) -> Result<RpcBlockHeader, ProvingTaskError> {
        let header = self
            .cl_client
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

    /// Retrieves the evm_ee block commitment corresponding to the given L2 block ID
    pub(crate) async fn get_exec_commitment(
        &self,
        cl_block_id: L2BlockId,
    ) -> Result<EvmEeBlockCommitment, ProvingTaskError> {
        let header = self.get_l2_block_header(cl_block_id).await?;
        let block = self.evm_ee_operator.get_block(header.block_idx).await?;

        Ok(EvmEeBlockCommitment::new(
            block.header.number,
            Buf32(block.header.hash.into()),
        ))
    }

    /// Retrieves the [`Chainstate`] before the given blocks is applied
    pub(crate) async fn get_chainstate_before(
        &self,
        blkid: L2BlockId,
    ) -> Result<Chainstate, ProvingTaskError> {
        let raw_witness: Vec<u8> = self
            .cl_client
            .get_cl_block_witness_raw(blkid)
            .await
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;
        let (chainstate, _): (Chainstate, L2Block) =
            borsh::from_slice(&raw_witness).expect("invalid witness");
        Ok(chainstate)
    }

    /// Retrieves the [`L2Block`] for the given id
    pub(crate) async fn get_block(&self, blkid: &L2BlockId) -> Result<L2Block, ProvingTaskError> {
        let raw_witness: Vec<u8> = self
            .cl_client
            .get_cl_block_witness_raw(*blkid)
            .await
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?;
        let (_, blk): (Chainstate, L2Block) =
            borsh::from_slice(&raw_witness).expect("invalid witness");
        Ok(blk)
    }
}

#[derive(Debug)]
pub(crate) struct ClStfParams {
    pub epoch: u64,
    pub l2_range: (L2BlockCommitment, L2BlockCommitment),
    pub l1_range: Option<(L1BlockCommitment, L1BlockCommitment)>,
}

impl ProvingOp for ClStfOperator {
    type Program = ClStfProgram;
    type Params = ClStfParams;

    fn construct_proof_ctx(&self, range: &Self::Params) -> Result<ProofContext, ProvingTaskError> {
        let ClStfParams { l2_range, .. } = range;

        let (start, end) = l2_range;
        // Do some sanity checks
        assert!(
            start.slot() <= end.slot(),
            "failed to construct CL STF proof context. start_slot: {} > end_slot {}",
            start.slot(),
            end.slot()
        );

        Ok(ProofContext::ClStf(*start, *end))
    }

    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        db: &ProofDb,
    ) -> Result<ClStfInput, ProvingTaskError> {
        let (start_block, end_block) = match task_id.context() {
            ProofContext::ClStf(start, end) => (*start, *end),
            _ => return Err(ProvingTaskError::InvalidInput("CL_STF".to_string())),
        };

        let deps = db
            .get_proof_deps(*task_id.context())
            .map_err(ProvingTaskError::DatabaseError)?
            .ok_or(ProvingTaskError::DependencyNotFound(*task_id))?;

        // sanity check
        // CL STF can have at most 2 deps
        // 1. It will always have EVM EE Proof as a dependency
        // 2. If the CL STF includes terminal block, it also has BTC Blockspace Proof as a
        //    dependency
        assert!(deps.len() <= 2, "invalid CL STF deps");

        // First dependency is always EVM EE Proof
        let evm_ee_id = deps.first().ok_or(ProvingTaskError::NoTasksFound)?;
        let evm_ee_key = ProofKey::new(*evm_ee_id, *task_id.host());
        let evm_ee_proof = db
            .get_proof(&evm_ee_key)
            .map_err(ProvingTaskError::DatabaseError)?
            .ok_or(ProvingTaskError::ProofNotFound(evm_ee_key))?;
        let evm_ee_vk = get_verification_key(&evm_ee_key);
        let evm_ee_proof_with_vk = (evm_ee_proof, evm_ee_vk);

        // Second dependency that is optional is BTC Blockspace Proof
        let btc_blockspace_proof_with_vk = deps
            .get(1)
            .map(|btc_blockspace_id| -> Result<_, ProvingTaskError> {
                let btc_blockspace_key = ProofKey::new(*btc_blockspace_id, *task_id.host());
                let btc_blockspace_proof = db
                    .get_proof(&btc_blockspace_key)
                    .map_err(ProvingTaskError::DatabaseError)?
                    .ok_or(ProvingTaskError::ProofNotFound(evm_ee_key))?;
                let btc_blockspace_vk = get_verification_key(&btc_blockspace_key);
                Ok((btc_blockspace_proof, btc_blockspace_vk))
            })
            .transpose()?;

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

        let rollup_params = self.rollup_params.as_ref().clone();
        Ok(ClStfInput {
            rollup_params,
            chainstate,
            l2_blocks,
            evm_ee_proof_with_vk,
            btc_blockspace_proof_with_vk,
        })
    }

    async fn create_deps_tasks(
        &self,
        params: Self::Params,
        db: &ProofDb,
        task_tracker: Arc<Mutex<TaskTracker>>,
    ) -> Result<Vec<ProofKey>, ProvingTaskError> {
        let ClStfParams {
            epoch,
            l1_range,
            l2_range,
        } = params;

        let el_start_block = self.get_exec_commitment(*l2_range.0.blkid()).await?;
        let el_end_block = self.get_exec_commitment(*l2_range.1.blkid()).await?;

        let mut tasks = self
            .evm_ee_operator
            .create_task((el_start_block, el_end_block), task_tracker.clone(), db)
            .await?;

        if let Some(l1_range) = l1_range {
            let btc_params = BtcBlockscanParams {
                epoch,
                range: l1_range,
            };
            let btc_tasks = self
                .btc_blockspace_operator
                .create_task(btc_params, task_tracker, db)
                .await?;
            tasks.extend(btc_tasks);
        }

        Ok(tasks)
    }
}
