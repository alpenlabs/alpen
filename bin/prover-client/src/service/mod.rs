//! Service layer for Prover Service integration
//!
//! This module provides ProofHandler implementations for all proof types,
//! bridging between Prover Service and the operators.
//!
//! ## Structure
//!
//! - `task` - ProofTask type and ProgramType implementation
//! - `adapters` - Adapters for integrating operators with paas traits
//! - `handlers` - ProofHandler implementations for each proof type
//! - `task_store` - Persistent task storage using Sled
//! - `host_resolver` - Centralized zkVM host resolution (single source of truth)

use strata_paas::ZkVmBackend;
use strata_primitives::proof::{ProofContext, ProofKey, ProofZkVm};

mod adapters;
mod handlers;
mod host_resolver;
mod task;
mod task_store;

// Re-export public types
pub(crate) use handlers::{new_checkpoint_handler, new_cl_stf_handler, new_evm_ee_stf_handler};
pub(crate) use task::{ProofContextVariant, ProofTask};
pub(crate) use task_store::SledTaskStore;

// ============================================================================
// Backend Resolution - Unified API
// ============================================================================

/// Get the current zkVM backend based on feature flags
///
/// Returns `ZkVmBackend::SP1` if the `sp1` feature is enabled, otherwise `Native`.
/// Use this when submitting tasks to Prover Service.
///
/// Delegates to the centralized host_resolver module.
///
/// # Example
/// ```ignore
/// let backend = zkvm_backend();
/// prover_handle.submit_task(task, backend).await?;
/// ```
#[inline]
pub(crate) fn zkvm_backend() -> ZkVmBackend {
    host_resolver::default_backend()
}

/// Create a ProofKey for the given ProofContext using the current backend
///
/// This is the primary way to create ProofKeys in the prover-client.
/// It automatically determines the correct zkVM type based on feature flags.
///
/// # Example
/// ```ignore
/// let proof_key = proof_key_for(ProofContext::Checkpoint(42));
/// let proof = db.get_proof(&proof_key)?;
/// ```
#[inline]
pub(crate) fn proof_key_for(proof_ctx: ProofContext) -> ProofKey {
    let zkvm = match zkvm_backend() {
        ZkVmBackend::SP1 => ProofZkVm::SP1,
        ZkVmBackend::Native => ProofZkVm::Native,
        ZkVmBackend::Risc0 => panic!("Risc0 backend is not supported"),
    };
    ProofKey::new(proof_ctx, zkvm)
}
