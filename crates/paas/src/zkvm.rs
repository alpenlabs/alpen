//! Zkaleido integration for PaaS
//!
//! This module provides an opinionated integration with zkaleido's `ZkVmProgram`
//! and `ZkVmHost` abstractions, allowing you to define proving tasks using zkaleido
//! programs.
//!
//! ## Example Usage
//!
//! Note: This module provides low-level zkaleido integration. Most users should use the
//! registry-based API from `strata_paas::registry` which provides better type safety and
//! extensibility.
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use strata_paas::zkvm::*;
//!
//! // 1. Define your program identifiers
//! #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
//! enum MyProgram {
//!     Checkpoint(u64),
//!     ClStf(u64, u64),
//!     EvmEeStf(u64),
//! }
//!
//! impl ProgramId for MyProgram {
//!     fn name(&self) -> String {
//!         match self {
//!             MyProgram::Checkpoint(idx) => format!("checkpoint_{}", idx),
//!             MyProgram::ClStf(start, end) => format!("cl_stf_{}_{}", start, end),
//!             MyProgram::EvmEeStf(block) => format!("evm_ee_stf_{}", block),
//!         }
//!     }
//! }
//!
//! // 2. Implement InputFetcher to define how to get inputs for your programs
//! struct MyInputFetcher {
//!     // Your data sources (RPC clients, databases, etc.)
//! }
//!
//! impl InputFetcher<MyProgram> for MyInputFetcher {
//!     type Program = CheckpointProgram;  // Your zkaleido program type
//!
//!     async fn fetch_input(&self, program: &MyProgram) -> PaaSResult<CheckpointInput> {
//!         match program {
//!             MyProgram::Checkpoint(idx) => {
//!                 // Fetch checkpoint input data
//!                 Ok(/* ... */)
//!             }
//!             // ... other programs
//!         }
//!     }
//! }
//!
//! // 3. For a complete example using the registry pattern (recommended),
//! // see the documentation in `strata_paas::registry` module.
//! ```

use std::marker::PhantomData;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use zkaleido::ZkVmProgram;

use crate::error::{PaaSError, PaaSResult};
use crate::Prover;

/// ZkVm backend identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZkVmBackend {
    /// Native execution (no proving)
    Native,
    /// SP1 prover
    SP1,
    /// RISC0 prover
    Risc0,
}

/// A proving task that uses a zkaleido ZkVmProgram
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(bound = "P: ProgramId")]
pub struct ZkVmTaskId<P: ProgramId> {
    /// The program identifier
    pub program: P,
    /// The backend to use for proving
    pub backend: ZkVmBackend,
}

/// Trait for identifying zkaleido programs
///
/// Implement this trait to define your program identifiers. This allows
/// you to have multiple different programs in the same PaaS instance.
pub trait ProgramId: Clone + Eq + std::hash::Hash + std::fmt::Debug + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static {
    /// Get a human-readable name for this program
    fn name(&self) -> String;
}

/// Trait for fetching inputs for zkaleido programs
///
/// The caller implements this trait to define how to fetch inputs for
/// their zkaleido programs.
pub trait InputFetcher<P: ProgramId>: Send + Sync + 'static {
    /// The zkaleido program type
    type Program: zkaleido::ZkVmProgram<Input: Send>;

    /// Fetch the input for a given task
    ///
    /// This is called by the worker when it's ready to prove a task.
    fn fetch_input(
        &self,
        program: &P,
    ) -> impl std::future::Future<Output = PaaSResult<<Self::Program as zkaleido::ZkVmProgram>::Input>> + Send;
}

/// Trait for storing proof results
///
/// The caller implements this trait to define how to store completed proofs.
pub trait ProofStore<P: ProgramId>: Send + Sync + 'static {
    /// Store a completed proof
    fn store_proof(
        &self,
        task_id: &ZkVmTaskId<P>,
        proof: zkaleido::ProofReceiptWithMetadata,
    ) -> impl std::future::Future<Output = PaaSResult<()>> + Send;
}

/// ZkVm prover that integrates with zkaleido
///
/// This implements the generic `Prover` trait and provides zkaleido-specific
/// proving functionality.
pub struct ZkVmProver<P, I, S, H>
where
    P: ProgramId,
    I: InputFetcher<P>,
    S: ProofStore<P>,
    H: zkaleido::ZkVmHost,
{
    /// Input fetcher
    input_fetcher: Arc<I>,
    /// Proof store
    proof_store: Arc<S>,
    /// ZkVm host for proving
    host: Arc<H>,
    _phantom: PhantomData<P>,
}

impl<P, I, S, H> ZkVmProver<P, I, S, H>
where
    P: ProgramId,
    I: InputFetcher<P>,
    S: ProofStore<P>,
    H: zkaleido::ZkVmHost,
{
    /// Create a new ZkVm prover
    pub fn new(
        input_fetcher: Arc<I>,
        proof_store: Arc<S>,
        host: Arc<H>,
    ) -> Self {
        Self {
            input_fetcher,
            proof_store,
            host,
            _phantom: PhantomData,
        }
    }
}

impl<P, I, S, H> Prover for ZkVmProver<P, I, S, H>
where
    P: ProgramId,
    I: InputFetcher<P>,
    S: ProofStore<P>,
    H: zkaleido::ZkVmHost,
{
    type TaskId = ZkVmTaskId<P>;
    type Backend = ZkVmBackend;

    fn backend(&self, task_id: &Self::TaskId) -> Self::Backend {
        task_id.backend.clone()
    }

    async fn prove(&self, task_id: Self::TaskId) -> PaaSResult<()> {
        // Fetch input
        let input = self
            .input_fetcher
            .fetch_input(&task_id.program)
            .await?;

        // Prove using zkaleido
        let proof = I::Program::prove(&input, self.host.as_ref())
            .map_err(|e| PaaSError::PermanentFailure(format!("Proving failed: {}", e)))?;

        // Store the proof
        self.proof_store.store_proof(&task_id, proof).await?;

        Ok(())
    }
}
