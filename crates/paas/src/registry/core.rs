//! Dynamic registry for program handlers with full type erasure
//!
//! This module provides a registry-based architecture that allows multiple
//! program types to be registered and dispatched dynamically without exposing
//! discriminants in the public API.

use std::{
    any::Any,
    collections::HashMap,
    future::Future,
    hash::Hash,
    marker::PhantomData,
    pin::Pin,
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use zkaleido::{ProofReceiptWithMetadata, ZkVmProgram};

use crate::error::{PaaSError, PaaSResult};
use crate::ZkVmBackend;

/// Trait that program types must implement for dynamic dispatch
///
/// This trait allows PaaS to extract a routing key from any program type
/// without the user having to specify it explicitly.
pub trait ProgramType:
    Clone +
    Eq +
    Hash +
    Send +
    Sync +
    std::fmt::Debug +
    Serialize +
    for<'de> Deserialize<'de> +
    'static
{
    /// Routing key type (usually an enum discriminant)
    type RoutingKey: Eq + Hash + Clone + Send + Sync + std::fmt::Debug + 'static;

    /// Extract routing key for handler lookup
    fn routing_key(&self) -> Self::RoutingKey;
}

/// Type-erased input container
pub struct BoxedInput(pub Box<dyn Any + Send + Sync>);

impl BoxedInput {
    /// Create a new boxed input
    pub fn new<T: Send + Sync + 'static>(input: T) -> Self {
        Self(Box::new(input))
    }

    /// Downcast to concrete type
    pub fn downcast<T: 'static>(self) -> Result<Box<T>, PaaSError> {
        self.0
            .downcast::<T>()
            .map_err(|_| PaaSError::PermanentFailure("Type mismatch in BoxedInput".to_string()))
    }

    /// Downcast reference to concrete type
    pub fn downcast_ref<T: 'static>(&self) -> Result<&T, PaaSError> {
        self.0
            .downcast_ref::<T>()
            .ok_or_else(|| PaaSError::PermanentFailure("Type mismatch in BoxedInput".to_string()))
    }
}

/// Type-erased proof container
pub struct BoxedProof(pub Box<dyn Any + Send + Sync>);

impl BoxedProof {
    /// Create a new boxed proof
    pub fn new<T: Send + Sync + 'static>(proof: T) -> Self {
        Self(Box::new(proof))
    }

    /// Downcast to concrete type
    pub fn downcast<T: 'static>(self) -> Result<Box<T>, PaaSError> {
        self.0
            .downcast::<T>()
            .map_err(|_| PaaSError::PermanentFailure("Type mismatch in BoxedProof".to_string()))
    }
}

/// Handler for a specific program variant (object-safe)
pub trait ProgramHandler<P: ProgramType>: Send + Sync + 'static {
    /// Fetch input for this program
    fn fetch_input<'a>(
        &'a self,
        program: &'a P,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<BoxedInput>> + Send + 'a>>;

    /// Prove with the given backend
    fn prove<'a>(
        &'a self,
        input: BoxedInput,
        backend: &'a ZkVmBackend,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<BoxedProof>> + Send + 'a>>;

    /// Store the completed proof
    fn store_proof<'a>(
        &'a self,
        program: &'a P,
        proof: BoxedProof,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<()>> + Send + 'a>>;
}

/// Registry that routes programs to their handlers
pub struct ProgramRegistry<P: ProgramType> {
    handlers: HashMap<P::RoutingKey, Arc<dyn ProgramHandler<P>>>,
}

impl<P: ProgramType> ProgramRegistry<P> {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler for a specific program variant
    pub fn register(
        &mut self,
        key: P::RoutingKey,
        handler: Arc<dyn ProgramHandler<P>>,
    ) {
        self.handlers.insert(key, handler);
    }

    /// Get handler for a program (using routing key)
    pub fn get_handler(&self, program: &P) -> Option<&Arc<dyn ProgramHandler<P>>> {
        let key = program.routing_key();
        self.handlers.get(&key)
    }

    /// Fetch input using the appropriate handler
    pub async fn fetch_input(&self, program: &P) -> PaaSResult<BoxedInput> {
        let handler = self
            .get_handler(program)
            .ok_or_else(|| {
                PaaSError::PermanentFailure(format!(
                    "No handler registered for program: {:?}",
                    program
                ))
            })?;

        handler.fetch_input(program).await
    }

    /// Prove using the appropriate handler
    pub async fn prove(
        &self,
        program: &P,
        input: BoxedInput,
        backend: &ZkVmBackend,
    ) -> PaaSResult<BoxedProof> {
        let handler = self
            .get_handler(program)
            .ok_or_else(|| {
                PaaSError::PermanentFailure(format!(
                    "No handler registered for program: {:?}",
                    program
                ))
            })?;

        handler.prove(input, backend).await
    }

    /// Store proof using the appropriate handler
    pub async fn store_proof(
        &self,
        program: &P,
        proof: BoxedProof,
    ) -> PaaSResult<()> {
        let handler = self
            .get_handler(program)
            .ok_or_else(|| {
                PaaSError::PermanentFailure(format!(
                    "No handler registered for program: {:?}",
                    program
                ))
            })?;

        handler.store_proof(program, proof).await
    }
}

impl<P: ProgramType> Default for ProgramRegistry<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: ProgramType> Clone for ProgramRegistry<P> {
    fn clone(&self) -> Self {
        Self {
            handlers: self.handlers.clone(),
        }
    }
}

/// Bridge between concrete types and trait objects
///
/// This struct knows the concrete types and handles the type erasure
/// required for dynamic dispatch.
pub struct ConcreteHandler<P, Prog, I, S, H>
where
    P: ProgramType,
    Prog: ZkVmProgram,
    I: InputProvider<P, Prog>,
    S: ProofStore<P>,
    H: zkaleido::ZkVmHost + Send + Sync + 'static,
{
    input_provider: Arc<I>,
    proof_store: Arc<S>,
    host: Arc<H>,
    _phantom: PhantomData<(P, Prog)>,
}

impl<P, Prog, I, S, H> ConcreteHandler<P, Prog, I, S, H>
where
    P: ProgramType,
    Prog: ZkVmProgram,
    I: InputProvider<P, Prog>,
    S: ProofStore<P>,
    H: zkaleido::ZkVmHost + Send + Sync + 'static,
{
    /// Create a new concrete handler with a specific host
    pub fn new(input_provider: Arc<I>, proof_store: Arc<S>, host: Arc<H>) -> Self {
        Self {
            input_provider,
            proof_store,
            host,
            _phantom: PhantomData,
        }
    }
}

impl<P, Prog, I, S, H> ProgramHandler<P> for ConcreteHandler<P, Prog, I, S, H>
where
    P: ProgramType,
    Prog: ZkVmProgram + Send + Sync + 'static,
    Prog::Input: Send + Sync + 'static,
    I: InputProvider<P, Prog>,
    S: ProofStore<P>,
    H: zkaleido::ZkVmHost + Send + Sync + 'static,
{
    fn fetch_input<'a>(
        &'a self,
        program: &'a P,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<BoxedInput>> + Send + 'a>> {
        Box::pin(async move {
            let input = self.input_provider.provide_input(program).await?;
            Ok(BoxedInput::new(input))
        })
    }

    fn prove<'a>(
        &'a self,
        input: BoxedInput,
        _backend: &'a ZkVmBackend,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<BoxedProof>> + Send + 'a>> {
        Box::pin(async move {
            // Downcast to concrete input type
            let concrete_input = input.downcast::<Prog::Input>()?;

            // Prove using the stored host
            // The backend parameter is ignored - the host determines the backend
            let proof = Prog::prove(&concrete_input, self.host.as_ref())
                .map_err(|e| PaaSError::PermanentFailure(format!("Proving failed: {}", e)))?;

            Ok(BoxedProof::new(proof))
        })
    }

    fn store_proof<'a>(
        &'a self,
        program: &'a P,
        proof: BoxedProof,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<()>> + Send + 'a>> {
        Box::pin(async move {
            // Downcast to concrete proof type
            let concrete_proof = proof.downcast::<ProofReceiptWithMetadata>()?;

            self.proof_store.store_proof(program, *concrete_proof).await
        })
    }
}

/// Trait for providing inputs for a specific zkVM program
pub trait InputProvider<P, Prog>: Send + Sync + 'static
where
    P: ProgramType,
    Prog: ZkVmProgram,
{
    /// Provide input for the given program
    fn provide_input<'a>(
        &'a self,
        program: &'a P,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<Prog::Input>> + Send + 'a>>;
}

/// Trait for storing completed proofs
pub trait ProofStore<P>: Send + Sync + 'static
where
    P: ProgramType,
{
    /// Store a completed proof
    fn store_proof<'a>(
        &'a self,
        program: &'a P,
        proof: ProofReceiptWithMetadata,
    ) -> Pin<Box<dyn Future<Output = PaaSResult<()>> + Send + 'a>>;
}
