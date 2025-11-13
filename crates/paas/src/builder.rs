//! Builder for creating prover service instances

use std::sync::Arc;

use strata_service::ServiceBuilder;
use strata_tasks::TaskExecutor;

use crate::config::PaaSConfig;
use crate::error::{PaaSError, PaaSResult};
use crate::handle::ProverHandle;
use crate::service::ProverService;
use crate::state::ProverServiceState;
use crate::zkvm::{InputFetcher, ProgramId, ProofStore, ZkVmBackend, ZkVmProver};

/// Builder for ProverService with zkaleido integration
///
/// Generic over:
/// - `P`: Program identifier type (implements `ProgramId`)
/// - `I`: Input fetcher (implements `InputFetcher<P>`)
/// - `S`: Proof store (implements `ProofStore<P>`)
/// - `H`: ZkVM host (implements `zkaleido::ZkVmHost`)
pub struct ProverServiceBuilder<P, I, S, H>
where
    P: ProgramId,
    I: InputFetcher<P>,
    S: ProofStore<P>,
    H: zkaleido::ZkVmHost,
{
    input_fetcher: Option<Arc<I>>,
    proof_store: Option<Arc<S>>,
    host: Option<Arc<H>>,
    config: Option<PaaSConfig<ZkVmBackend>>,
    _phantom: std::marker::PhantomData<P>,
}

impl<P, I, S, H> ProverServiceBuilder<P, I, S, H>
where
    P: ProgramId,
    I: InputFetcher<P>,
    S: ProofStore<P>,
    H: zkaleido::ZkVmHost,
{
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            input_fetcher: None,
            proof_store: None,
            host: None,
            config: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Set the input fetcher
    pub fn with_input_fetcher(mut self, input_fetcher: Arc<I>) -> Self {
        self.input_fetcher = Some(input_fetcher);
        self
    }

    /// Set the proof store
    pub fn with_proof_store(mut self, proof_store: Arc<S>) -> Self {
        self.proof_store = Some(proof_store);
        self
    }

    /// Set the zkVM host
    pub fn with_host(mut self, host: Arc<H>) -> Self {
        self.host = Some(host);
        self
    }

    /// Set the configuration
    pub fn with_config(mut self, config: PaaSConfig<ZkVmBackend>) -> Self {
        self.config = Some(config);
        self
    }

    /// Launch the service
    pub async fn launch(self, executor: &TaskExecutor) -> PaaSResult<ProverHandle<P>> {
        // Validate dependencies
        let input_fetcher = self
            .input_fetcher
            .ok_or_else(|| PaaSError::Config("InputFetcher not set".into()))?;
        let proof_store = self
            .proof_store
            .ok_or_else(|| PaaSError::Config("ProofStore not set".into()))?;
        let host = self
            .host
            .ok_or_else(|| PaaSError::Config("ZkVmHost not set".into()))?;
        let config = self
            .config
            .ok_or_else(|| PaaSError::Config("Config not set".into()))?;

        // Create the zkaleido prover
        let prover = Arc::new(ZkVmProver::new(input_fetcher, proof_store, host));

        // Create service state
        let service_state = ProverServiceState::new(prover, config);

        // Create service builder
        let mut service_builder =
            ServiceBuilder::<ProverService<ZkVmProver<P, I, S, H>>, _>::new()
                .with_state(service_state);

        // Create command handle
        let command_handle = service_builder.create_command_handle(100);

        // Launch service
        let monitor = service_builder
            .launch_async("prover", executor)
            .await
            .map_err(PaaSError::Internal)?;

        // Return handle
        Ok(ProverHandle::new(command_handle, monitor))
    }
}

impl<P, I, S, H> Default for ProverServiceBuilder<P, I, S, H>
where
    P: ProgramId,
    I: InputFetcher<P>,
    S: ProofStore<P>,
    H: zkaleido::ZkVmHost,
{
    fn default() -> Self {
        Self::new()
    }
}
