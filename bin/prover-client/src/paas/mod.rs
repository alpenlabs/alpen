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

mod task;
mod fetchers;
mod store;

// Re-export public types
pub(crate) use fetchers::{CheckpointFetcher, ClStfFetcher, EvmEeFetcher};
pub(crate) use store::ProofStoreService;
pub(crate) use task::{ProofContextVariant, ProofTask};

/// Convert ZkVmBackend to ProofZkVm
pub(crate) fn backend_to_zkvm(backend: strata_paas::ZkVmBackend) -> strata_primitives::proof::ProofZkVm {
    match backend {
        strata_paas::ZkVmBackend::SP1 => strata_primitives::proof::ProofZkVm::SP1,
        strata_paas::ZkVmBackend::Native => strata_primitives::proof::ProofZkVm::Native,
        strata_paas::ZkVmBackend::Risc0 => panic!("Risc0 not supported"),
    }
}

/// Get the current backend based on feature flags
pub(crate) fn get_current_backend() -> strata_primitives::proof::ProofZkVm {
    #[cfg(feature = "sp1")]
    {
        strata_primitives::proof::ProofZkVm::SP1
    }
    #[cfg(not(feature = "sp1"))]
    {
        strata_primitives::proof::ProofZkVm::Native
    }
}
