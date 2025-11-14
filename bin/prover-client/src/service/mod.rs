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

/// Get the current backend for PaaS operations
///
/// Returns `ZkVmBackend::SP1` if the `sp1` feature is enabled, otherwise `Native`.
/// Use this when interacting with PaaS APIs.
#[inline]
pub(crate) fn current_paas_backend() -> ZkVmBackend {
    #[cfg(feature = "sp1")]
    {
        ZkVmBackend::SP1
    }
    #[cfg(not(feature = "sp1"))]
    {
        ZkVmBackend::Native
    }
}

/// Get the current zkVM for proof key creation
///
/// Returns `ProofZkVm::SP1` if the `sp1` feature is enabled, otherwise `Native`.
/// Use this when creating ProofKeys or working with the database.
#[inline]
pub(crate) fn current_zkvm() -> ProofZkVm {
    #[cfg(feature = "sp1")]
    {
        ProofZkVm::SP1
    }
    #[cfg(not(feature = "sp1"))]
    {
        ProofZkVm::Native
    }
}

/// Convert PaaS backend to zkVM type
///
/// # Panics
/// Panics if `backend` is `Risc0` as it's not supported.
#[inline]
pub(crate) fn paas_backend_to_zkvm(backend: &ZkVmBackend) -> ProofZkVm {
    match backend {
        ZkVmBackend::SP1 => ProofZkVm::SP1,
        ZkVmBackend::Native => ProofZkVm::Native,
        ZkVmBackend::Risc0 => panic!("Risc0 backend is not supported"),
    }
}
