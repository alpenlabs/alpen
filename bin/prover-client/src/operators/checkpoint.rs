use std::sync::Arc;

use jsonrpsee::http_client::HttpClient;
use strata_db::traits::ProofDatabase;
use strata_primitives::{
    l1::L1BlockCommitment,
    l2::L2BlockCommitment,
    proof::{ProofContext, ProofKey},
};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use strata_rocksdb::prover::db::ProofDb;
use strata_rpc_api::StrataApiClient;
use strata_rpc_types::{RpcCheckpointConfStatus, RpcCheckpointInfo};
use strata_zkvm_hosts::get_verification_key;
use tokio::sync::Mutex;
use tracing::{error, info};

use super::{cl_stf::ClStfOperator, ProvingOp};
use crate::{
    checkpoint_runner::{errors::CheckpointResult, submit::submit_checkpoint_proof},
    errors::ProvingTaskError,
    operators::cl_stf::ClStfParams,
    task_tracker::TaskTracker,
};

/// A struct that implements the [`ProvingOp`] for Checkpoint Proof.
///
/// It is responsible for managing the data and tasks required to generate Checkpoint Proof. It
/// fetches the necessary inputs for the [`CheckpointProgram`] by:
// TODO: update docstring here
#[derive(Debug, Clone)]
pub(crate) struct CheckpointOperator {
    cl_client: HttpClient,
    cl_stf_operator: Arc<ClStfOperator>,
    enable_checkpoint_runner: bool,
}

impl CheckpointOperator {
    /// Creates a new BTC operations instance.
    pub(crate) fn new(
        cl_client: HttpClient,
        cl_stf_operator: Arc<ClStfOperator>,
        enable_checkpoint_runner: bool,
    ) -> Self {
        Self {
            cl_client,
            cl_stf_operator,
            enable_checkpoint_runner,
        }
    }

    /// Creates dependency tasks for the given `checkpoint_info`.
    ///
    /// # Arguments
    ///
    /// - `checkpoint_info`: checkpoint data.
    /// - `db`: A reference to the proof database.
    /// - `task_tracker`: A shared task tracker for managing task dependencies.
    ///
    /// # Returns
    ///
    /// A [`Vec`] containing the [`ProofKey`] for the dependent proving operations.
    async fn create_deps_tasks_inner(
        &self,
        checkpoint_info: RpcCheckpointInfo,
        db: &ProofDb,
        task_tracker: Arc<Mutex<TaskTracker>>,
    ) -> Result<Vec<ProofKey>, ProvingTaskError> {
        let ckp_idx = checkpoint_info.idx;
        let l2_blocks_len = checkpoint_info.l2_range.1.slot() - checkpoint_info.l2_range.0.slot();
        info!(%ckp_idx, %l2_blocks_len);

        // Since the L1Manifests are only included on the terminal block of epoch transition, we can
        // strategize to split the L2 Blocks as following:
        // 1. Prove CL terminal block separately
        // 2. Split other blocks on a on chunks of 20?
        // TODO: add better heuristic to split, so that it is most efficient
        // Since the EVM EE STF will be the heaviest, the splitting can be done based on that
        //
        // For now, do everything on a single chunk
        let cl_stf_params = ClStfParams {
            epoch: checkpoint_info.idx,
            l1_range: Some(checkpoint_info.l1_range),
            l2_range: checkpoint_info.l2_range,
        };
        self.cl_stf_operator
            .create_task(cl_stf_params, task_tracker, db)
            .await
    }

    /// Manual creation of checkpoint task. Intended to be used in tests.
    ///
    /// #Note
    ///
    /// This is analogous to [`ProvingOp::create_task`].
    /// In fact, a forked version with construction of dependency tasks from manually constructed
    /// [`RpcCheckpointInfo`].
    ///
    /// # Arguments
    ///
    /// - `checkpoint_idx`: index of the checkpoint.
    /// - `l1_range`: range of blocks on L1 to be included in the checkpoint.
    /// - `l2_range`: range of block on L2 to be included in the  checkpoint.
    /// - `task_tracker`: A shared task tracker for managing task dependencies.
    /// - `db`: A reference to the proof database.
    ///
    /// # Returns
    ///
    /// A vector of [`ProofKey`] corresponding to the checkpoint proving operation.
    pub(crate) async fn create_task_raw(
        &self,
        checkpoint_idx: u64,
        l1_range: (L1BlockCommitment, L1BlockCommitment),
        l2_range: (L2BlockCommitment, L2BlockCommitment),
        task_tracker: Arc<Mutex<TaskTracker>>,
        db: &ProofDb,
    ) -> Result<Vec<ProofKey>, ProvingTaskError> {
        let checkpoint_info = RpcCheckpointInfo {
            idx: checkpoint_idx,
            l1_range,
            l2_range,
            l1_reference: None,
            confirmation_status: RpcCheckpointConfStatus::Pending,
        };
        let proof_ctx = self.construct_proof_ctx(&checkpoint_idx)?;

        // Try to fetch the existing prover tasks for dependencies.
        let proof_deps = db
            .get_proof_deps(proof_ctx)
            .map_err(ProvingTaskError::DatabaseError)?;

        let deps_ctx = match proof_deps {
            // Reuse the existing dependency tasks fetched from DB.
            Some(v) => v,
            // Create new dependency tasks.
            None => {
                let deps_keys = self
                    .create_deps_tasks_inner(checkpoint_info, db, task_tracker.clone())
                    .await?;
                let deps: Vec<_> = deps_keys.iter().map(|v| v.context().to_owned()).collect();

                if !deps.is_empty() {
                    db.put_proof_deps(proof_ctx, deps.clone())
                        .map_err(ProvingTaskError::DatabaseError)?;
                }
                deps
            }
        };

        let mut task_tracker = task_tracker.lock().await;
        task_tracker.create_tasks(proof_ctx, deps_ctx, db)
    }

    async fn fetch_ckp_info(&self, ckp_idx: u64) -> Result<RpcCheckpointInfo, ProvingTaskError> {
        self.cl_client
            .get_checkpoint_info(ckp_idx)
            .await
            .inspect_err(|_| error!(%ckp_idx, "Failed to fetch CheckpointInfo"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?
            .ok_or(ProvingTaskError::WitnessNotFound)
    }

    /// Retrieves the latest checkpoint index
    pub(crate) async fn fetch_latest_ckp_idx(&self) -> Result<u64, ProvingTaskError> {
        self.cl_client
            .get_latest_checkpoint_index(None)
            .await
            .inspect_err(|_| error!("Failed to fetch latest checkpoint"))
            .map_err(|e| ProvingTaskError::RpcError(e.to_string()))?
            .ok_or(ProvingTaskError::WitnessNotFound)
    }

    /// Returns a reference to the internal CL (Consensus Layer) [`HttpClient`].
    pub(crate) const fn cl_client(&self) -> &HttpClient {
        &self.cl_client
    }

    pub(crate) async fn submit_checkpoint_proof(
        &self,
        checkpoint_index: u64,
        proof_key: &ProofKey,
        proof_db: &ProofDb,
    ) -> CheckpointResult<()> {
        if !self.enable_checkpoint_runner {
            return Ok(());
        }
        submit_checkpoint_proof(checkpoint_index, self.cl_client(), proof_key, proof_db).await
    }
}

impl ProvingOp for CheckpointOperator {
    type Program = CheckpointProgram;
    type Params = u64;

    fn construct_proof_ctx(
        &self,
        ckp_idx: &Self::Params,
    ) -> Result<ProofContext, ProvingTaskError> {
        Ok(ProofContext::Checkpoint(*ckp_idx))
    }

    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        db: &ProofDb,
    ) -> Result<CheckpointProverInput, ProvingTaskError> {
        let deps = db
            .get_proof_deps(*task_id.context())
            .map_err(ProvingTaskError::DatabaseError)?
            .ok_or(ProvingTaskError::DependencyNotFound(*task_id))?;

        assert!(!deps.is_empty(), "checkpoint must have some CL STF proofs");

        let cl_stf_key = ProofKey::new(deps[0], *task_id.host());
        let cl_stf_vk = get_verification_key(&cl_stf_key);

        let mut cl_stf_proofs = Vec::with_capacity(deps.len());
        for dep in deps {
            match dep {
                ProofContext::ClStf(..) => {}
                _ => panic!("invalid"),
            };
            let cl_stf_key = ProofKey::new(dep, *task_id.host());
            let proof = db
                .get_proof(&cl_stf_key)
                .map_err(ProvingTaskError::DatabaseError)?
                .ok_or(ProvingTaskError::ProofNotFound(cl_stf_key))?;
            cl_stf_proofs.push(proof);
        }

        Ok(CheckpointProverInput {
            cl_stf_proofs,
            cl_stf_vk,
        })
    }

    async fn create_deps_tasks(
        &self,
        ckp_idx: Self::Params,
        db: &ProofDb,
        task_tracker: Arc<Mutex<TaskTracker>>,
    ) -> Result<Vec<ProofKey>, ProvingTaskError> {
        let checkpoint_info = self.fetch_ckp_info(ckp_idx).await?;
        self.create_deps_tasks_inner(checkpoint_info, db, task_tracker)
            .await
    }
}
