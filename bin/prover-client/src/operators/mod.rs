//! A module defining traits and operations for proof generation using ZKVMs.
//!
//! This module provides the [`ProvingOp`] trait, which encapsulates the lifecycle of proof
//! generation tasks. Each proof generation task includes fetching necessary proof dependencies,
//! creating tasks, fetching inputs, and performing the proof computation using various supported
//! ZKVMs.
//!
//! The operations are designed to interact with a [`ProofDb`] for storing and retrieving proofs,
//! a [`TaskTracker`] for managing task dependencies, and [`ZkVmHost`] host for ZKVM-specific
//! computations.
//!
//! Supported ZKVMs:
//!
//! - Native
//! - SP1 (requires `sp1` feature enabled)
//! - Risc0 (requires `risc0` feature enabled)

use std::sync::Arc;

use strata_db::traits::ProofDatabase;
use strata_primitives::proof::{ProofContext, ProofKey};
use strata_rocksdb::prover::db::ProofDb;
use tokio::sync::Mutex;
use tracing::{error, info, instrument};
use zkaleido::{ZkVmHost, ZkVmProgram};

use crate::{errors::ProvingTaskError, task_tracker::TaskTracker};

pub(crate) mod btc;
pub(crate) mod checkpoint;
pub(crate) mod cl_stf;
pub(crate) mod evm_ee;
pub(crate) mod operator;

pub(crate) use operator::ProofOperator;

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
    async fn create_task(
        &self,
        params: Self::Params,
        task_tracker: Arc<Mutex<TaskTracker>>,
        db: &ProofDb,
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
        task_tracker.create_tasks(proof_ctx, deps_ctx, db)
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
    #[allow(unused_variables)]
    async fn create_deps_tasks(
        &self,
        params: Self::Params,
        db: &ProofDb,
        task_tracker: Arc<Mutex<TaskTracker>>,
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
        db: &ProofDb,
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
        db: &ProofDb,
        host: &impl ZkVmHost,
    ) -> Result<(), ProvingTaskError> {
        info!("Starting proof generation");

        let input = self
            .fetch_input(task_id, db)
            .await
            .inspect_err(|e| error!(?e, "Failed to fetch input"))?;

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_primitives::{buf::Buf32, evm_exec::EvmEeBlockCommitment, l1::L1BlockCommitment};
    use strata_rocksdb::{prover::db::ProofDb, test_utils::get_rocksdb_tmp_instance_for_prover};
    use strata_rpc_types::ProofKey;
    use tokio::sync::Mutex;
    use zkaleido::ZkVmProgram;

    use super::ProvingOp;
    use crate::{errors::ProvingTaskError, status::ProvingTaskStatus, task_tracker::TaskTracker};

    // Test stub of zkaleido::ZkVmProver.
    struct TestProgram;

    impl ZkVmProgram for TestProgram {
        type Input = u64;

        type Output = String;

        fn name() -> String {
            "test_prover".to_string()
        }

        fn proof_type() -> zkaleido::ProofType {
            zkaleido::ProofType::Compressed
        }

        fn prepare_input<'a, B>(_input: &'a Self::Input) -> zkaleido::ZkVmInputResult<B::Input>
        where
            B: zkaleido::ZkVmInputBuilder<'a>,
        {
            todo!()
        }

        fn process_output<H>(
            _public_values: &zkaleido::PublicValues,
        ) -> zkaleido::ZkVmResult<Self::Output>
        where
            H: zkaleido::ZkVmHost,
        {
            Ok("test output".to_string())
        }
    }

    /// Grandparent proving ops that has [`ParentOps`] as a dependency.
    /// The full dependency graph for proving ops:
    /// [`GrandparentOps`] (this) -> [`ParentOps`] -> [`ChildOps`]
    struct GrandparentOps;

    impl ProvingOp for GrandparentOps {
        type Program = TestProgram;

        type Params = u64;

        fn construct_proof_ctx(
            &self,
            params: &Self::Params,
        ) -> Result<strata_primitives::proof::ProofContext, crate::errors::ProvingTaskError>
        {
            Ok(strata_primitives::proof::ProofContext::Checkpoint(*params))
        }

        async fn fetch_input(
            &self,
            _task_id: &strata_rpc_types::ProofKey,
            _db: &strata_rocksdb::prover::db::ProofDb,
        ) -> Result<<Self::Program as zkaleido::ZkVmProgram>::Input, crate::errors::ProvingTaskError>
        {
            todo!()
        }

        async fn create_deps_tasks(
            &self,
            params: Self::Params,
            db: &ProofDb,
            task_tracker: Arc<Mutex<TaskTracker>>,
        ) -> Result<Vec<ProofKey>, ProvingTaskError> {
            let child = ParentOps;
            child.create_task(params, task_tracker, db).await
        }
    }

    /// Parent proving ops that has [`ChildOps`] as a dependency.
    /// The full dependency graph for proving ops:
    /// [`GrandparentOps`] -> [`ParentOps`] (this) -> [`ChildOps`]
    struct ParentOps;

    impl ProvingOp for ParentOps {
        type Program = TestProgram;

        type Params = u64;

        fn construct_proof_ctx(
            &self,
            params: &Self::Params,
        ) -> Result<strata_primitives::proof::ProofContext, crate::errors::ProvingTaskError>
        {
            let mut batch = Buf32::default();
            batch.0[0] = *params as u8;
            Ok(strata_primitives::proof::ProofContext::EvmEeStf(
                EvmEeBlockCommitment::new(*params, Buf32::default()),
                EvmEeBlockCommitment::new(*params, Buf32::default()),
            ))
        }

        async fn fetch_input(
            &self,
            _task_id: &strata_rpc_types::ProofKey,
            _db: &strata_rocksdb::prover::db::ProofDb,
        ) -> Result<<Self::Program as zkaleido::ZkVmProgram>::Input, crate::errors::ProvingTaskError>
        {
            todo!()
        }

        async fn create_deps_tasks(
            &self,
            params: Self::Params,
            db: &ProofDb,
            task_tracker: Arc<Mutex<TaskTracker>>,
        ) -> Result<Vec<ProofKey>, ProvingTaskError> {
            let child = ChildOps;
            child.create_task(params, task_tracker, db).await
        }
    }

    // Child proving ops that has no dependencies.
    // The full dependency graph for proving ops:
    /// [`GrandparentOps`] -> [`ParentOps`] -> [`ChildOps`] (this)
    struct ChildOps;

    impl ProvingOp for ChildOps {
        type Program = TestProgram;

        type Params = u64;

        fn construct_proof_ctx(
            &self,
            params: &Self::Params,
        ) -> Result<strata_primitives::proof::ProofContext, crate::errors::ProvingTaskError>
        {
            Ok(strata_primitives::proof::ProofContext::BtcBlockspace(
                0,
                L1BlockCommitment::new(*params, Buf32::default().into()),
                L1BlockCommitment::new(*params, Buf32::default().into()),
            ))
        }

        async fn fetch_input(
            &self,
            _task_id: &strata_rpc_types::ProofKey,
            _db: &strata_rocksdb::prover::db::ProofDb,
        ) -> Result<<Self::Program as zkaleido::ZkVmProgram>::Input, crate::errors::ProvingTaskError>
        {
            todo!()
        }
    }

    fn setup_db() -> ProofDb {
        let (db, db_ops) = get_rocksdb_tmp_instance_for_prover().unwrap();
        ProofDb::new(db, db_ops)
    }

    #[tokio::test]
    async fn test_success_ops() {
        let tracker = Arc::new(Mutex::new(TaskTracker::new()));
        let db = setup_db();

        // Create a single grandparent proving op, assert it waits for dependencies.
        let grand_ops = GrandparentOps {};
        let create_res = grand_ops.create_task(12, tracker.clone(), &db).await;
        let proving_keys = create_res.unwrap();
        let pkey = proving_keys.first().unwrap();
        assert_eq!(
            *tracker.lock().await.get_task(*pkey).unwrap(),
            ProvingTaskStatus::WaitingForDependencies
        );

        // Create another one.
        let create_res = grand_ops.create_task(117, tracker.clone(), &db).await;
        create_res.unwrap();

        // Create a child op and assert it's pending.
        let child_ops = ChildOps {};
        let create_res = child_ops.create_task(29, tracker.clone(), &db).await;
        let proving_keys = create_res.unwrap();
        let pkey = proving_keys.first().unwrap();
        assert_eq!(
            *tracker.lock().await.get_task(*pkey).unwrap(),
            ProvingTaskStatus::Pending
        );
    }

    #[tokio::test]
    async fn test_fail_creation_same_ops() {
        let tracker = Arc::new(Mutex::new(TaskTracker::new()));
        let db = setup_db();

        // Create a single grandparent proving op.
        let grand_ops = GrandparentOps {};
        let res = grand_ops.create_task(117, tracker.clone(), &db).await;
        res.unwrap();

        // Create another grandparent proving op with the same input, assert it fails.
        let res = grand_ops.create_task(117, tracker.clone(), &db).await;
        assert!(matches!(res, Err(ProvingTaskError::TaskAlreadyFound(_))));
    }

    #[tokio::test]
    async fn test_prover_client_restart() {
        let tracker = Arc::new(Mutex::new(TaskTracker::new()));
        let db = setup_db();

        // Create two grandparent proving op.
        let grand_ops = GrandparentOps {};
        let res = grand_ops.create_task(12, tracker.clone(), &db).await;
        res.unwrap();
        let res = grand_ops.create_task(117, tracker.clone(), &db).await;
        res.unwrap();

        // Emulate the prover-client restart:
        // 1. Clear the internal state of the task tracker.
        // 2. Leave the DB state (the task deps table) as is.
        tracker.lock().await.clear_state();

        // Create the already existing grandparent proving op.
        let res = grand_ops.create_task(12, tracker.clone(), &db).await;
        let proving_keys = res.unwrap();
        let pkey = proving_keys.first().unwrap();
        // Expect that a task is successfully inserted as waiting for dependencies.
        assert_eq!(
            *tracker.lock().await.get_task(*pkey).unwrap(),
            ProvingTaskStatus::WaitingForDependencies
        );
        // Expect that a child task in a pending state.
        assert!(!tracker
            .lock()
            .await
            .get_tasks_by_status(|status| matches!(status, ProvingTaskStatus::Pending))
            .is_empty(),);

        let parent_ops = ParentOps {};
        // Create a parent proving op with the same input - it should fail, because the
        // corresponding task already got inserted.
        let res = parent_ops.create_task(12, tracker.clone(), &db).await;
        assert!(matches!(res, Err(ProvingTaskError::TaskAlreadyFound(_))));
    }
}
