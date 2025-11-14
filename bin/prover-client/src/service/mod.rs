//! Service layer for PaaS integration
//!
//! This module bridges between PaaS (which works with ProofTask) and the
//! operators (which work with ProofContext), implementing the input provisioning
//! and proof storage for the PaaS service.
//!
//! ## Structure
//!
//! - `task` - ProofTask type and ProgramType implementation
//! - `inputs` - InputProvider implementations for each program type
//! - `store` - ProofStore implementation for proof persistence

use strata_paas::ZkVmBackend;
use strata_primitives::proof::ProofZkVm;

mod inputs;
mod store;
mod task;

// Re-export public types
pub(crate) use inputs::{
    CheckpointInputProvider, ClStfInputProvider, EvmEeInputProvider,
};
pub(crate) use store::ProofStoreService;
pub(crate) use task::{ProofContextVariant, ProofTask};

// ============================================================================
// Backend Resolution - Unified API
// ============================================================================

/// Get the current zkVM backend based on feature flags
///
/// Returns `ZkVmBackend::SP1` if the `sp1` feature is enabled, otherwise `Native`.
/// Use this when interacting with PaaS APIs or submitting tasks.
///
/// # Example
/// ```ignore
/// let backend = zkvm_backend();
/// prover_handle.submit_task(task, backend).await?;
/// ```
#[inline]
pub(crate) fn zkvm_backend() -> ZkVmBackend {
    #[cfg(feature = "sp1")]
    {
        ZkVmBackend::SP1
    }
    #[cfg(not(feature = "sp1"))]
    {
        ZkVmBackend::Native
    }
}

/// Convert ZkVmBackend to ProofZkVm for database operations
///
/// Use this when creating ProofKeys or interacting with the proof database.
/// Typically used as `backend_to_zkvm(&zkvm_backend())` to get the current zkVM type.
///
/// # Example
/// ```ignore
/// let backend = zkvm_backend();
/// let zkvm = backend_to_zkvm(&backend);
/// let proof_key = ProofKey::new(proof_ctx, zkvm);
/// db.get_proof(&proof_key)?;
/// ```
///
/// # Panics
/// Panics if `backend` is `Risc0` as it's not supported.
#[inline]
pub(crate) fn backend_to_zkvm(backend: &ZkVmBackend) -> ProofZkVm {
    match backend {
        ZkVmBackend::SP1 => ProofZkVm::SP1,
        ZkVmBackend::Native => ProofZkVm::Native,
        ZkVmBackend::Risc0 => panic!("Risc0 backend is not supported"),
    }
}

/// Macro to resolve zkVM host based on proof context variant and feature flags
///
/// Takes a `ProofContextVariant` and returns an `Arc<Host>` for the appropriate
/// backend (SP1 or Native) based on compile-time feature flags.
///
/// # Example
/// ```ignore
/// use crate::service::ProofContextVariant;
/// let host = resolve_host!(ProofContextVariant::Checkpoint);
/// ```
#[macro_export]
macro_rules! resolve_host {
    ($variant:expr) => {{
        // Create a sample ProofContext for host initialization
        let ctx = match $variant {
            $crate::service::ProofContextVariant::Checkpoint => {
                strata_primitives::proof::ProofContext::Checkpoint(0)
            }
            $crate::service::ProofContextVariant::ClStf => {
                let null = strata_primitives::l2::L2BlockCommitment::null();
                strata_primitives::proof::ProofContext::ClStf(null, null)
            }
            $crate::service::ProofContextVariant::EvmEeStf => {
                let null = strata_primitives::evm_exec::EvmEeBlockCommitment::null();
                strata_primitives::proof::ProofContext::EvmEeStf(null, null)
            }
        };

        // Resolve host based on feature flags
        #[cfg(feature = "sp1")]
        {
            strata_zkvm_hosts::sp1::get_host(&ctx) // Already returns Arc<SP1Host>
        }
        #[cfg(not(feature = "sp1"))]
        {
            std::sync::Arc::from(strata_zkvm_hosts::native::get_host(&ctx))
        }
    }};
}
