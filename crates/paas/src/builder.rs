//! Builder for constructing and launching prover service

use std::sync::Arc;

use strata_db::traits::ProofDatabase;
use strata_service::ServiceBuilder;
use strata_tasks::TaskExecutor;
use tokio::sync::watch;

use crate::{
    PaaSConfig, PaaSError, PaaSStatus, handle::ProverHandle, service::ProverService,
    state::ProverServiceState,
};

/// Builder for constructing and launching the prover service
///
/// # Example
///
/// ```no_run
/// use strata_paas::{ProverBuilder, PaaSConfig};
/// # use std::sync::Arc;
/// # use strata_db_store_sled::prover::ProofDBSled;
///
/// # async fn example(db: Arc<ProofDBSled>, executor: strata_tasks::TaskExecutor) -> anyhow::Result<()> {
/// let handle = ProverBuilder::new()
///     .with_config(PaaSConfig::default())
///     .with_database(db)
///     .launch(&executor)?;
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct ProverBuilder<D: ProofDatabase> {
    config: Option<PaaSConfig>,
    database: Option<Arc<D>>,
}

impl<D: ProofDatabase> ProverBuilder<D> {
    /// Creates a new builder
    pub fn new() -> Self {
        Self {
            config: None,
            database: None,
        }
    }

    /// Sets the configuration
    pub fn with_config(mut self, config: PaaSConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Sets the proof database
    pub fn with_database(mut self, database: Arc<D>) -> Self {
        self.database = Some(database);
        self
    }

    /// Validates that all required dependencies are set
    fn validate(&self) -> Result<(), PaaSError> {
        if self.config.is_none() {
            return Err(PaaSError::MissingDependency("config"));
        }
        if self.database.is_none() {
            return Err(PaaSError::MissingDependency("database"));
        }
        Ok(())
    }

    /// Launches the prover service
    ///
    /// Returns a handle for interacting with the service.
    pub async fn launch(self, executor: &TaskExecutor) -> Result<ProverHandle<D>, PaaSError> {
        // Validate dependencies
        self.validate()?;

        let config = self.config.unwrap();
        let database = self.database.unwrap();

        // Create status channel
        let (status_tx, status_rx) = watch::channel(PaaSStatus {
            active_tasks: 0,
            queued_tasks: 0,
            completed_tasks: 0,
            failed_tasks: 0,
            worker_utilization: 0.0,
        });

        // Create service state
        let state = ProverServiceState::new(config, database, status_tx);

        // Build service using ServiceBuilder
        let mut service_builder = ServiceBuilder::<ProverService<D>, _>::new().with_state(state);

        // Create command handle (64 is channel buffer size)
        let command_handle = service_builder.create_command_handle(64);

        // Launch the service
        let _service_monitor = service_builder
            .launch_async("prover_service", executor)
            .await
            .map_err(|e| PaaSError::LaunchFailed(format!("{:?}", e)))?;

        // Return handle
        Ok(ProverHandle::new(command_handle, status_rx))
    }
}

impl<D: ProofDatabase> Default for ProverBuilder<D> {
    fn default() -> Self {
        Self::new()
    }
}
