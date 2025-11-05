//! A module defining traits and operations for proof generation using ZKVMs.
//!
//! This module provides the [`ProvingOp`] trait, which encapsulates the lifecycle of proof
//! generation tasks. Each proof generation task includes fetching necessary proof dependencies,
//! creating tasks, fetching inputs, and performing the proof computation using various supported
//! ZKVMs.
//!
//! The operations are designed to interact with a [`ProofDatabase`] for storing and retrieving
//! proofs, a [`TaskTracker`] for managing task dependencies, and [`ZkVmHost`] host for
//! ZKVM-specific computations.
//!
//! Supported ZKVMs:
//!
//! - Native
//! - SP1 (requires `sp1` feature enabled)

use std::sync::Arc;

use strata_db::traits::ProofDatabase;
use strata_db_store_sled::prover::ProofDBSled;
use strata_primitives::proof::{ProofContext, ProofKey};
use tokio::sync::Mutex;
use tracing::{error, info, instrument};
use zkaleido::{ZkVmHost, ZkVmProgram};

use crate::errors::ProvingTaskError;

pub(crate) mod checkpoint;
pub(crate) mod cl_stf;
pub(crate) mod evm_ee;
pub(crate) mod operator;

pub(crate) use operator::ProofOperator;

/// Trait for task trackers (implemented by both TaskTracker and TaskTrackerAdapter)
pub(crate) trait TaskTrackerLike: Send + Sync {
    async fn create_tasks(
        &mut self,
        proof_id: strata_primitives::proof::ProofContext,
        deps: Vec<strata_primitives::proof::ProofContext>,
        db: &ProofDBSled,
    ) -> Result<Vec<ProofKey>, ProvingTaskError>;
}

/// A trait defining the operations required for proof generation.
///
/// This trait outlines the steps for proof generation tasks, including fetching proof dependencies,
/// creating tasks, fetching inputs for the prover, and executing the proof computation using
/// supported ZKVMs.
pub(crate) trait ProvingOp {
    /// The program type associated with this operation, implementing the [`ZkVmProgram`] trait.
    type Program: ZkVmProgram;

    /// Parameters required for this operation.
    ///
    /// The `Params` type is designed to be easy to understand, such as a block height for Bitcoin
    /// blockspace proofs. The `fetch_proof_context` method converts these simple parameters
    /// into more detailed `ProofContext`, which includes all the necessary information (e.g.,
    /// block hash) to generate proofs.
    type Params;

    /// Fetches the proof contexts and their dependencies for the specified parameters.
    ///
    /// # Arguments
    ///
    /// - `params`: The parameters specific to the operation.
    /// - `task_tracker`: A shared task tracker for managing task dependencies.
    /// - `db`: A reference to the proof database.
    ///
    /// # Returns
    ///
    /// A vector of [`ProofKey`] corresponding to a given proving operation.
    async fn create_task<T: TaskTrackerLike>(
        &self,
        params: Self::Params,
        task_tracker: Arc<Mutex<T>>,
        db: &ProofDBSled,
    ) -> Result<Vec<ProofKey>, ProvingTaskError> {
        let proof_ctx = self.construct_proof_ctx(&params)?;

        // Try to fetch the existing prover tasks for dependencies.
        let proof_deps = db
            .get_proof_deps(proof_ctx)
            .map_err(ProvingTaskError::DatabaseError)?;

        let deps_ctx = {
            // Create proving dependency tasks.
            let deps_keys = self
                .create_deps_tasks(params, db, task_tracker.clone())
                .await?;
            let deps: Vec<_> = deps_keys.iter().map(|v| v.context().to_owned()).collect();

            // Only insert deps into DB if any and not in the DB already.
            if !deps.is_empty() && proof_deps.is_none() {
                db.put_proof_deps(proof_ctx, deps.clone())
                    .map_err(ProvingTaskError::DatabaseError)?;
            }
            deps
        };

        let mut task_tracker = task_tracker.lock().await;
        task_tracker.create_tasks(proof_ctx, deps_ctx, db).await
    }

    /// Construct [`ProofContext`] from the proving operation parameters.
    fn construct_proof_ctx(&self, params: &Self::Params) -> Result<ProofContext, ProvingTaskError>;

    /// Creates a set of dependency tasks.
    ///
    /// # Important
    ///
    /// The default impl defines no dependencies, so certain [`ProvingOp`] with dependencies
    /// should "override" it.
    ///
    /// # Arguments
    ///
    /// - `params`: The parameters specific to the operation.
    /// - `task_tracker`: A shared task tracker for managing task dependencies.
    /// - `db`: A reference to the proof database.
    ///
    /// # Returns
    ///
    /// A [`Vec`] containing the [`ProofKey`] for the dependent proving operations.
    #[expect(unused_variables, reason = "used for overriding default impl")]
    async fn create_deps_tasks<T: TaskTrackerLike>(
        &self,
        params: Self::Params,
        db: &ProofDBSled,
        task_tracker: Arc<Mutex<T>>,
    ) -> Result<Vec<ProofKey>, ProvingTaskError> {
        Ok(vec![])
    }

    /// Fetches the input required for the proof computation.
    ///
    /// # Arguments
    ///
    /// - `task_id`: The key representing the proof task.
    /// - `db`: A reference to the proof database.
    ///
    /// # Returns
    ///
    /// The input required by the prover for the specified task.
    async fn fetch_input(
        &self,
        task_id: &ProofKey,
        db: &ProofDBSled,
    ) -> Result<<Self::Program as ZkVmProgram>::Input, ProvingTaskError>;

    /// Executes the proof computation for the specified task.
    ///
    /// # Arguments
    ///
    /// - `task_id`: The key representing the proof task.
    /// - `db`: A reference to the proof database.
    ///
    /// # Returns
    ///
    /// An empty result if the proof computation is successful.
    #[instrument(skip(self, db, host), fields(task_id = ?task_id))]
    async fn prove(
        &self,
        task_id: &ProofKey,
        db: &ProofDBSled,
        host: &impl ZkVmHost,
    ) -> Result<(), ProvingTaskError> {
        info!("Starting proof generation");

        // Failing to fetch_input is somewhat expected -
        // exex sometimes lags behind the block production.
        // Logs with info to not pollute the logs with false positives.
        let input = self
            .fetch_input(task_id, db)
            .await
            .inspect_err(|e| info!(?e, "Failed to fetch input"))?;

        let proof_res = <Self::Program as ZkVmProgram>::prove(&input, host);

        match &proof_res {
            Ok(_) => {
                info!("Proof generated successfully")
            }
            Err(e) => {
                error!(?e, "Failed to generate proof")
            }
        }

        let proof = proof_res.map_err(ProvingTaskError::ZkVmError)?;

        db.put_proof(*task_id, proof)
            .map_err(ProvingTaskError::DatabaseError)?;

        Ok(())
    }
}
