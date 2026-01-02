//! Generic remote proof handler implementation
//!
//! This module provides `RemoteProofHandler`, a generic implementation of `ProofHandler`
//! that works with any zkaleido program supporting remote proving (SP1, Risc0, etc.).
//!
//! The handler is parameterized over:
//! - Input fetcher: Provides proof inputs
//! - Proof storer: Persists completed proofs
//! - Host resolver: Resolves zkVM hosts for different backends
//! - Program: The zkaleido program to execute
//!
//! ## Architecture
//!
//! `RemoteProofHandler` uses the `HostResolver` trait from the `host` module to obtain
//! zkVM hosts, then executes proofs using the appropriate execution strategy for each backend.

use std::{any, fmt, marker::PhantomData, time};

use async_trait::async_trait;
use strata_tasks::TaskExecutor;
use tokio::{task, time::sleep};
use tracing::info;
use zkaleido::{ProofReceiptWithMetadata, ZkVmProgram, ZkVmRemoteProgram};

use crate::{
    error::{ProverServiceError, ProverServiceResult},
    handler::{host::HostResolver, BoxedInput, InputFetcher, ProofHandler, ProofStorer},
    program::ProgramType,
    ZkVmBackend,
};

// ============================================================================
// Remote Proof Handler
// ============================================================================

/// Generic remote proof handler using zkaleido traits
///
/// This handler works with any input fetcher, proof storer, and host resolver,
/// and any zkaleido program implementing `ZkVmProgram + ZkVmRemoteProgram`.
///
/// # Type Parameters
///
/// - `P`: Program type (implements `ProgramType`)
/// - `F`: Input fetcher (implements `InputFetcher`)
/// - `S`: Proof storer (implements `ProofStorer`)
/// - `R`: Host resolver (implements `HostResolver`)
/// - `Prog`: ZkVM program (implements `ZkVmProgram + ZkVmRemoteProgram`)
///
/// # Example
///
/// ```ignore
/// type MyHandler = RemoteProofHandler<
///     MyProgramType,
///     MyInputFetcher,
///     MyProofStorer,
///     MyHostResolver,
///     MyZkVmProgram,
/// >;
///
/// let handler = MyHandler::new(
///     fetcher,
///     storer,
///     resolver,
///     executor,
/// );
/// ```
#[derive(Clone)]
pub struct RemoteProofHandler<P, F, S, R, Prog> {
    fetcher: F,
    storer: S,
    resolver: R,
    executor: TaskExecutor,
    _phantom: PhantomData<(P, Prog)>,
}

impl<P, F, S, R, Prog> fmt::Debug for RemoteProofHandler<P, F, S, R, Prog> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteProofHandler")
            .field("fetcher", &any::type_name::<F>())
            .field("storer", &any::type_name::<S>())
            .field("resolver", &any::type_name::<R>())
            .finish()
    }
}

impl<P, F, S, R, Prog> RemoteProofHandler<P, F, S, R, Prog>
where
    P: ProgramType,
    F: InputFetcher<P>,
    S: ProofStorer<P>,
    R: HostResolver<P>,
    Prog: ZkVmProgram + ZkVmRemoteProgram,
{
    /// Create a new remote proof handler
    ///
    /// # Arguments
    ///
    /// * `fetcher` - Input fetcher for retrieving proof inputs
    /// * `storer` - Proof storer for persisting completed proofs
    /// * `resolver` - Host resolver for resolving zkVM hosts
    /// * `executor` - Task executor for executing proofs
    pub fn new(fetcher: F, storer: S, resolver: R, executor: TaskExecutor) -> Self {
        Self {
            fetcher,
            storer,
            resolver,
            executor,
            _phantom: PhantomData,
        }
    }
}

#[async_trait]
impl<P, F, S, R, Prog> ProofHandler<P> for RemoteProofHandler<P, F, S, R, Prog>
where
    P: ProgramType + Send + Sync + 'static,
    F: InputFetcher<P> + Clone + 'static,
    F::Input: Into<Prog::Input> + 'static,
    F::Error: 'static,
    S: ProofStorer<P> + Clone + 'static,
    S::Error: 'static,
    R: HostResolver<P> + Clone + 'static,
    Prog: ZkVmProgram + ZkVmRemoteProgram + Send + Sync + 'static,
    Prog::Input: Send + Sync + 'static,
{
    async fn fetch_input(&self, program: &P) -> ProverServiceResult<BoxedInput> {
        let input = self
            .fetcher
            .fetch_input(program)
            .await
            .map_err(|e| ProverServiceError::TransientFailure(e.to_string()))?;

        Ok(BoxedInput::new(input))
    }

    async fn execute_proof(
        &self,
        program: &P,
        input: BoxedInput,
        backend: &ZkVmBackend,
    ) -> ProverServiceResult<ProofReceiptWithMetadata> {
        let backend = backend.clone();
        let runtime_handle = self.executor.handle().clone();
        let program_clone = program.clone();
        let resolver = self.resolver.clone();

        info!("Executing proof with backend {:?}", backend);

        // Downcast input to fetcher type, then convert to program input type
        let fetcher_input = *input.downcast::<F::Input>()?;
        let program_input: Prog::Input = fetcher_input.into();

        // Execute with proper !Send future isolation for SP1
        // Layer 1: spawn_blocking - move to blocking thread pool
        // Layer 2: block_on - re-enter async runtime
        // Layer 3-4-5: LocalSet for !Send futures (Remote only)
        let result = task::spawn_blocking(move || {
            runtime_handle.block_on(async move {
                // Resolve host once using unified API
                let host = resolver.resolve(&program_clone, &backend);

                match backend {
                    // Remote backends (SP1, Risc0, etc.) use async polling
                    ZkVmBackend::SP1 | ZkVmBackend::Risc0 => {
                        // Remote backends may produce !Send futures, so use LocalSet
                        let local = task::LocalSet::new();
                        local
                            .run_until(async move {
                                // Start remote proving
                                let proof_id = host
                                    .start_proving::<Prog>(&program_input)
                                    .await
                                    .map_err(|e| {
                                        ProverServiceError::PermanentFailure(format!(
                                            "Failed to start remote proving: {}",
                                            e
                                        ))
                                    })?;

                                // TODO: prettify.
                                // Poll for proof completion with exponential backoff
                                let mut interval = time::Duration::from_secs(1);
                                let max_interval = time::Duration::from_secs(30);

                                loop {
                                    sleep(interval).await;

                                    match host.get_proof_if_ready(proof_id.clone()).await {
                                        Ok(Some(proof)) => return Ok(proof),
                                        Ok(None) => {
                                            // Not ready yet, continue polling with backoff
                                            interval = (interval * 2).min(max_interval);
                                        }
                                        Err(e) => {
                                            return Err(ProverServiceError::TransientFailure(
                                                format!("Failed to retrieve proof: {}", e),
                                            ));
                                        }
                                    }
                                }
                            })
                            .await
                    }
                    // Native backend uses synchronous proving
                    ZkVmBackend::Native => {
                        // Native execution - blocking, doesn't need LocalSet
                        host.prove::<Prog>(&program_input)
                            .map_err(|e| ProverServiceError::PermanentFailure(e.to_string()))
                    }
                }
            })
        })
        .await
        .map_err(|e| {
            ProverServiceError::Internal(anyhow::anyhow!("spawn_blocking join error: {}", e))
        })??;

        Ok(result)
    }

    async fn store_proof(
        &self,
        program: &P,
        proof: ProofReceiptWithMetadata,
    ) -> ProverServiceResult<()> {
        self.storer
            .store_proof(program, proof)
            .await
            .map_err(|e| ProverServiceError::PermanentFailure(e.to_string()))
    }
}
