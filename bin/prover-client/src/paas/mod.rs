//! PaaS integration for prover-client
//!
//! This module bridges between PaaS (which works with ProofTask) and the
//! operators (which work with ProofContext), implementing the registry-based
//! PaaS API.
//!
//! ## Structure
//!
//! - `task` - ProofTask type and ProgramType implementation
//! - `fetchers` - InputFetcher implementations for each program type
//! - `store` - ProofStore implementation for proof persistence

use strata_paas::ZkVmBackend;
use strata_primitives::proof::ProofZkVm;

mod fetchers;
mod store;
mod task;

// Re-export public types
pub(crate) use fetchers::{CheckpointFetcher, ClStfFetcher, EvmEeFetcher};
pub(crate) use store::ProofStoreService;
pub(crate) use task::{ProofContextVariant, ProofTask};

/// Convert ZkVmBackend to ProofZkVm
pub(crate) fn backend_to_zkvm(backend: ZkVmBackend) -> ProofZkVm {
    match backend {
        ZkVmBackend::SP1 => ProofZkVm::SP1,
        ZkVmBackend::Native => ProofZkVm::Native,
        ZkVmBackend::Risc0 => panic!("Risc0 not supported"),
    }
}

/// Get the current backend based on feature flags
pub(crate) fn get_current_backend() -> ProofZkVm {
    #[cfg(feature = "sp1")]
    {
        ProofZkVm::SP1
    }
    #[cfg(not(feature = "sp1"))]
    {
        ProofZkVm::Native
    }
}
