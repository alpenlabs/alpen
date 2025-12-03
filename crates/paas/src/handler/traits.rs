//! Unified proof handler trait for all execution strategies
//!
//! This module defines the core ProofHandler trait that all program handlers
//! must implement. Handlers are responsible for:
//! 1. Fetching inputs for their program
//! 2. Executing proofs (hiding execution complexity like LocalSet)
//! 3. Storing completed proofs
//!
//! Handlers encapsulate all execution strategy details (!Send futures, blocking calls, etc.)
//!
//! ## Ergonomic Scaffolding Traits
//!
//! For easier implementation of ProofHandler, this module provides two scaffolding traits:
//! - `InputFetcher`: Fetch proof inputs from any source
//! - `ProofStorer`: Store completed proofs to any backend
//!
//! These traits allow building generic handlers like `RemoteProofHandler` that work
//! with any input source and storage backend.

use std::any::Any;

use async_trait::async_trait;
use zkaleido::ProofReceiptWithMetadata;

use crate::{error::ProverServiceResult, program::ProgramType, ZkVmBackend};

// ============================================================================
// Type-Erased Input Container
// ============================================================================

/// Type-erased input container for dynamic dispatch
///
/// Since `ProofHandler::fetch_input()` needs to work with any input type,
/// we use type erasure via `Box<dyn Any>`. The handler can then downcast
/// to the concrete type when executing the proof.
///
/// This allows PaaS to remain generic while handlers work with specific types.
pub struct BoxedInput(Box<dyn Any + Send + Sync>);

impl std::fmt::Debug for BoxedInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("BoxedInput").field(&"<any>").finish()
    }
}

impl BoxedInput {
    /// Create a new boxed input from any Send + Sync type
    pub fn new<T: Send + Sync + 'static>(input: T) -> Self {
        Self(Box::new(input))
    }

    /// Downcast to concrete type, consuming the box
    ///
    /// # Errors
    ///
    /// Returns an error if the type doesn't match.
    pub fn downcast<T: 'static>(self) -> Result<Box<T>, crate::ProverServiceError> {
        self.0.downcast::<T>().map_err(|_| {
            crate::ProverServiceError::PermanentFailure(
                "Type mismatch in BoxedInput downcast".to_string(),
            )
        })
    }

    /// Downcast reference to concrete type
    ///
    /// # Errors
    ///
    /// Returns an error if the type doesn't match.
    pub fn downcast_ref<T: 'static>(&self) -> Result<&T, crate::ProverServiceError> {
        self.0.downcast_ref::<T>().ok_or_else(|| {
            crate::ProverServiceError::PermanentFailure(
                "Type mismatch in BoxedInput downcast_ref".to_string(),
            )
        })
    }
}

// ============================================================================
// Handler Traits
// ============================================================================

/// Unified handler trait for proof generation
///
/// This trait defines the complete lifecycle of proof generation:
/// 1. fetch_input: Get the input data needed for proving
/// 2. execute_proof: Run the actual proof generation (may use LocalSet, spawn_blocking, etc.)
/// 3. store_proof: Persist the completed proof
///
/// Handlers own all execution complexity. ProverService orchestrates and controls capacity.
///
/// # Implementation Notes
///
/// - For async remote proving (SP1, R0): Use spawn_blocking + LocalSet internally
/// - For sync proving (Native): Use spawn_blocking directly
/// - **Capacity control is handled by the service** via semaphores before calling execute_proof
/// - Return errors as TransientFailure (retry) or PermanentFailure (don't retry)
/// - All methods must return Send futures (use spawn_blocking to handle !Send internally)
#[async_trait]
pub trait ProofHandler<P: ProgramType>: Send + Sync + 'static {
    /// Fetch input data for the given program
    ///
    /// This should retrieve whatever data is needed to execute the proof.
    /// Can be async I/O (database, RPC calls, etc.)
    async fn fetch_input(&self, program: &P) -> ProverServiceResult<BoxedInput>;

    /// Execute proof generation for the given input and backend
    ///
    /// This is where execution complexity lives:
    /// - For SP1 remote proving: spawn_blocking + block_on + LocalSet + prove
    /// - For Native: Just execute the program natively
    /// - For Risc0: spawn_blocking + blocking prove call
    ///
    /// **Note:** Capacity control (semaphore acquisition) is handled by the service
    /// before calling this method. Handlers should just focus on proof execution.
    ///
    /// The program is provided so handlers can access the full context (e.g., ProofContext for host
    /// selection).
    async fn execute_proof(
        &self,
        program: &P,
        input: BoxedInput,
        backend: &ZkVmBackend,
    ) -> ProverServiceResult<ProofReceiptWithMetadata>;

    /// Store the completed proof
    ///
    /// Persist the proof to whatever storage backend is configured.
    async fn store_proof(
        &self,
        program: &P,
        proof: ProofReceiptWithMetadata,
    ) -> ProverServiceResult<()>;
}

/// Trait for fetching proof inputs
///
/// Implement this trait to provide input data for proof generation from any source
/// (database, RPC, file system, etc.). The `RemoteProofHandler` uses this trait
/// to fetch inputs before executing proofs.
///
/// # Example
///
/// ```ignore
/// struct MyInputFetcher {
///     db: Arc<Database>,
/// }
///
/// #[async_trait]
/// impl InputFetcher<MyProgram> for MyInputFetcher {
///     type Input = MyInput;
///     type Error = DatabaseError;
///
///     async fn fetch_input(&self, program: &MyProgram) -> Result<Self::Input, Self::Error> {
///         self.db.get_input(program.id()).await
///     }
/// }
/// ```
#[async_trait]
pub trait InputFetcher<P: ProgramType>: Send + Sync {
    /// The type of input this fetcher produces
    type Input: Send + Sync;

    /// Error type for input fetching
    type Error: std::error::Error + Send + Sync + 'static;

    /// Fetch the input required for proof generation
    ///
    /// This method should retrieve all data needed to execute the proof.
    /// It may perform I/O operations like database queries or RPC calls.
    async fn fetch_input(&self, program: &P) -> Result<Self::Input, Self::Error>;
}

/// Trait for storing completed proofs
///
/// Implement this trait to persist proofs to any storage backend
/// (database, file system, S3, etc.). The `RemoteProofHandler` uses this trait
/// to store proofs after successful execution.
///
/// # Example
///
/// ```ignore
/// struct MyProofStorer {
///     db: Arc<Database>,
/// }
///
/// #[async_trait]
/// impl ProofStorer<MyProgram> for MyProofStorer {
///     type Error = DatabaseError;
///
///     async fn store_proof(
///         &self,
///         program: &MyProgram,
///         proof: ProofReceiptWithMetadata,
///     ) -> Result<(), Self::Error> {
///         self.db.save_proof(program.id(), proof).await
///     }
/// }
/// ```
#[async_trait]
pub trait ProofStorer<P: ProgramType>: Send + Sync {
    /// Error type for proof storage
    type Error: std::error::Error + Send + Sync + 'static;

    /// Store a completed proof
    ///
    /// This method should persist the proof to the configured storage backend.
    async fn store_proof(
        &self,
        program: &P,
        proof: ProofReceiptWithMetadata,
    ) -> Result<(), Self::Error>;
}
