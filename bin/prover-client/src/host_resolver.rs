//! Host resolution utilities for zkVM proving
//!
//! This module provides helpers to get sample contexts for host resolution.

use strata_primitives::proof::ProofContext;

/// Get a sample ProofContext for checkpoint (used for host initialization)
pub(crate) fn sample_checkpoint() -> ProofContext {
    ProofContext::Checkpoint(0)
}

/// Get a sample ProofContext for CL STF (used for host initialization)
pub(crate) fn sample_cl_stf() -> ProofContext {
    let null = strata_primitives::L2BlockCommitment::null();
    ProofContext::ClStf(null, null)
}

/// Get a sample ProofContext for EVM EE (used for host initialization)
pub(crate) fn sample_evm_ee() -> ProofContext {
    let null = strata_primitives::EvmEeBlockCommitment::null();
    ProofContext::EvmEeStf(null, null)
}

/// Macro to resolve host based on feature flags
#[macro_export]
macro_rules! resolve_host {
    ($ctx:expr) => {{
        #[cfg(feature = "sp1")]
        {
            std::sync::Arc::from(strata_zkvm_hosts::sp1::get_host(&$ctx))
        }
        #[cfg(not(feature = "sp1"))]
        {
            std::sync::Arc::from(strata_zkvm_hosts::native::get_host(&$ctx))
        }
    }};
}
