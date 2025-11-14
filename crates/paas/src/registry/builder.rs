//! Builder for registry-based prover service

use std::sync::Arc;

use strata_service::ServiceBuilder;
use strata_tasks::TaskExecutor;
use zkaleido::ZkVmProgram;

use crate::config::PaaSConfig;
use crate::error::PaaSResult;
use crate::service::ProverService;
use crate::state::ProverServiceState;
use crate::ZkVmBackend;
use crate::ProgramType;

use super::core::{ConcreteHandler, InputProvider, ProofStore, ProgramRegistry};
use super::handle::ProverHandle;
use super::prover::RegistryProver;

/// Builder for creating a prover service
///
/// This builder allows you to register multiple program handlers and then
/// launch a unified service that can handle all of them dynamically.
///
/// ## Example
///
/// ```rust,ignore
/// let handle = ProverServiceBuilder::new(config)
///     .with_program::<MyProgramA, _, _, _>(
///         MyProgramVariant::VariantA,
///         fetcher_a,
///         proof_store,
///         host_a,
///     )
///     .with_program::<MyProgramB, _, _, _>(
///         MyProgramVariant::VariantB,
///         fetcher_b,
///         proof_store,
///         host_b,
///     )
///     .launch(&executor)
///     .await?;
/// ```
pub struct ProverServiceBuilder<P: ProgramType> {
    registry: ProgramRegistry<P>,
    config: PaaSConfig<ZkVmBackend>,
}

impl<P: ProgramType> ProverServiceBuilder<P> {
    /// Create a new builder with the given configuration
    pub fn new(config: PaaSConfig<ZkVmBackend>) -> Self {
        Self {
            registry: ProgramRegistry::new(),
            config,
        }
    }

    /// Register a program handler
    ///
    /// This method registers a handler for a specific program variant identified by `key`.
    /// The `input_provider` and `proof_store` provide the concrete implementations for
    /// providing inputs and storing proofs for this program type.
    ///
    /// ## Type Parameters
    ///
    /// - `Prog`: The zkaleido `ZkVmProgram` type for this program
    /// - `I`: The input provider implementation
    /// - `S`: The proof store implementation
    /// - `H`: The zkVM host implementation
    ///
    /// ## Arguments
    ///
    /// - `key`: The routing key that identifies this program variant
    /// - `input_provider`: Implementation of `InputProvider` for providing inputs
    /// - `proof_store`: Implementation of `ProofStore` for storing proofs
    /// - `host`: The zkVM host to use for proving (e.g., SP1Host, NativeHost)
    pub fn with_program<Prog, I, S, H>(
        mut self,
        key: P::RoutingKey,
        input_provider: I,
        proof_store: S,
        host: Arc<H>,
    ) -> Self
    where
        Prog: ZkVmProgram + Send + Sync + 'static,
        Prog::Input: Send + Sync + 'static,
        I: InputProvider<P, Prog> + 'static,
        S: ProofStore<P> + 'static,
        H: zkaleido::ZkVmHost + Send + Sync + 'static,
    {
        let handler = ConcreteHandler::<P, Prog, I, S, H>::new(
            Arc::new(input_provider),
            Arc::new(proof_store),
            host,
        );

        self.registry.register(key, Arc::new(handler));
        self
    }

    /// Launch the prover service with all registered handlers
    ///
    /// This creates a prover with the registered handlers and launches
    /// the prover service.
    pub async fn launch(
        self,
        executor: &TaskExecutor,
    ) -> PaaSResult<ProverHandle<P>> {
        let prover = Arc::new(RegistryProver::new(Arc::new(self.registry)));
        let state = ProverServiceState::new(prover.clone(), self.config);

        // Create service builder
        let mut service_builder =
            ServiceBuilder::<ProverService<RegistryProver<P>>, _>::new()
                .with_state(state);

        // Create command handle
        let command_handle = service_builder.create_command_handle(100);

        // Launch service
        let monitor = service_builder
            .launch_async("prover", executor)
            .await
            .map_err(crate::error::PaaSError::Internal)?;

        // Return handle
        Ok(ProverHandle::new(command_handle, monitor))
    }
}
