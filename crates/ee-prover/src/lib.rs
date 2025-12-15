//! EE Prover Library
//!
//! This crate provides a high-level API for generating proofs for EE account updates.
//! It uses the `paas` framework internally for proof orchestration, retry logic, and
//! worker pool management.
//!
//! ## Example Usage
//!
//! ```ignore
//! use strata_ee_prover::{EeProverHandle, EeProverConfig};
//!
//! // Configure the prover
//! let config = EeProverConfig::new(sp1_workers, task_store, proof_storer)
//!     .with_retries(max_retries, base_delay, multiplier, max_delay);
//!
//! // Create the handle with a data provider
//! let prover = EeProverHandle::new(data_provider, genesis, config, &executor).await?;
//!
//! // Submit a proof task
//! let uuid = prover.prove_update(update_id).await?;
//!
//! // Check if proof is ready
//! if prover.is_ready(&uuid).await? {
//!     println!("Proof is ready!");
//! }
//! ```

mod host_resolver;
mod operator;

#[cfg(feature = "mock")]
pub mod mock;

// Re-export from proof-impl for convenience
pub use strata_proofimpl_eth_ee_acct::data_provider::EthEeAcctDataProvider;

// Use paas error types directly
pub use strata_paas::{ProverServiceError, ProverServiceResult as Result};

use std::{collections::HashMap, sync::Arc};

use serde::{Deserialize, Serialize};
use strata_paas::{
    HostResolver, InputFetcher, ProofStorer, ProgramType, ProverHandle, ProverServiceBuilder,
    ProverServiceConfig, RemoteProofHandler, RetryConfig, TaskStore, ZkVmBackend,
};
use strata_primitives::proof::ProofContext;
use strata_proofimpl_eth_ee_acct::EthEeAcctProgram;
use strata_tasks::TaskExecutor;

use crate::{
    host_resolver::EeHostResolver,
    operator::EthEeAcctOperator,
};

/// Wrapper type for [`ProofContext`] to implement [`ProgramType`]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EeProofTask(pub ProofContext);

impl From<ProofContext> for EeProofTask {
    fn from(ctx: ProofContext) -> Self {
        EeProofTask(ctx)
    }
}

impl From<EeProofTask> for ProofContext {
    fn from(task: EeProofTask) -> Self {
        task.0
    }
}

impl std::ops::Deref for EeProofTask {
    type Target = ProofContext;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Routing key for EE proof tasks
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EeProofVariant {
    EthEeAcct,
}

impl ProgramType for EeProofTask {
    type RoutingKey = EeProofVariant;

    fn routing_key(&self) -> Self::RoutingKey {
        match self.0 {
            ProofContext::EthEeAcct(_) => EeProofVariant::EthEeAcct,
            _ => panic!("EeProofTask only supports EthEeAcct variant"),
        }
    }
}

/// Configuration for the EE prover
#[derive(Clone)]
pub struct EeProverConfig<T, S> {
    /// Number of concurrent SP1 workers
    pub sp1_workers: usize,
    /// Task persistence store
    pub task_store: Arc<T>,
    /// Proof storage backend
    pub proof_storer: Arc<S>,
    /// Optional retry configuration
    pub retry: Option<RetryConfig>,
}

impl<T, S> EeProverConfig<T, S>
where
    T: TaskStore<EeProofTask> + Clone,
    S: ProofStorer<EeProofTask> + Clone,
{
    /// Create a new configuration with the given parameters
    pub fn new(sp1_workers: usize, task_store: Arc<T>, proof_storer: Arc<S>) -> Self {
        Self {
            sp1_workers,
            task_store,
            proof_storer,
            retry: None,
        }
    }

    /// Enable retries with custom configuration
    pub fn with_retries(
        mut self,
        max_retries: u32,
        base_delay_secs: u64,
        multiplier: f64,
        max_delay_secs: u64,
    ) -> Self {
        self.retry = Some(RetryConfig {
            max_retries,
            base_delay_secs,
            multiplier,
            max_delay_secs,
        });
        self
    }

    /// Convert to paas [`ProverServiceConfig`]
    fn to_paas_config(&self) -> ProverServiceConfig<ZkVmBackend> {
        let mut worker_counts = HashMap::new();
        worker_counts.insert(ZkVmBackend::SP1, self.sp1_workers);
        worker_counts.insert(ZkVmBackend::Native, 1);

        let mut config = ProverServiceConfig::new(worker_counts);
        config.retry = self.retry.clone();
        config
    }
}

/// Handle for interacting with the EE prover
///
/// This handle provides a high-level API for submitting proof tasks and checking their status.
pub struct EeProverHandle {
    inner: ProverHandle<EeProofTask>,
}

impl EeProverHandle {
    /// Create a new EE prover handle
    ///
    /// This initializes the paas prover service with the given configuration and launches
    /// it in the background.
    pub async fn new<D, T, S>(
        data_provider: Arc<D>,
        genesis: rsp_primitives::genesis::Genesis,
        config: EeProverConfig<T, S>,
        executor: &TaskExecutor,
    ) -> Result<Self>
    where
        D: EthEeAcctDataProvider + Clone + 'static,
        T: TaskStore<EeProofTask> + Clone + 'static,
        S: ProofStorer<EeProofTask> + Clone + 'static,
    {
        // Create operator with data provider
        let operator = EthEeAcctOperator::new(data_provider, genesis);

        // Create host resolver
        let resolver = EeHostResolver;

        // Create remote proof handler
        let handler = RemoteProofHandler::<
            EeProofTask,
            EthEeAcctOperator<D>,
            S,
            EeHostResolver,
            EthEeAcctProgram,
        >::new(
            operator,
            (*config.proof_storer).clone(),
            resolver,
            executor.clone(),
        );

        // Build and launch the prover service
        let mut builder = ProverServiceBuilder::new(config.to_paas_config())
            .with_task_store((*config.task_store).clone())
            .with_handler(EeProofVariant::EthEeAcct, Arc::new(handler));

        if let Some(retry_config) = &config.retry {
            builder = builder.with_retry_config(retry_config.clone());
        }

        let inner = builder.launch(executor).await?;

        Ok(Self { inner })
    }

    /// Submit a proof task for the given update ID
    ///
    /// Returns a UUID that can be used to track the proof generation progress.
    pub async fn prove_update(&self, update_id: u64) -> Result<String> {
        let proof_context = ProofContext::EthEeAcct(update_id);
        let task = EeProofTask(proof_context);

        self.inner.submit_task(task, ZkVmBackend::SP1).await
    }

    /// Check if a proof is ready (non-blocking)
    ///
    /// Returns true if the task is completed successfully, false otherwise.
    pub async fn is_ready(&self, uuid: &str) -> Result<bool> {
        let status = self.get_status(uuid).await?;
        Ok(matches!(status, strata_paas::TaskStatus::Completed))
    }

    /// Get the current status of a proof task
    pub async fn get_status(&self, uuid: &str) -> Result<strata_paas::TaskStatus> {
        self.inner.get_status(uuid).await
    }

    // TODO: Implement get_proof() once paas provides a proof retrieval mechanism
    // For now, users should query the proof_storer directly
}
