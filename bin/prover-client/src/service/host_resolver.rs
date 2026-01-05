//! Centralized zkVM host resolution
//!
//! This module provides the single source of truth for all zkVM host resolution
//! in prover-client. All feature flag logic, backend selection, and host instantiation
//! is centralized here.
//!
//! ## Design
//!
//! The `CentralizedHostResolver` implements the PaaS `HostResolver` trait, providing
//! a single `resolve()` method that determines the appropriate host based on:
//! - The proof context (which proof type: Checkpoint, ClStf, EvmEeStf)
//! - The backend (SP1, Native, Risc0)
//! - Feature flags (compile-time SP1 support)
//!
//! All host instantiation logic from `strata_zkvm_hosts` is called from here.

use strata_paas::{HostInstance, HostResolver, ZkVmBackend};
use strata_zkvm_hosts::native::get_host;
use zkaleido_native_adapter::NativeHost;
#[cfg(feature = "sp1")]
use zkaleido_sp1_host::SP1Host;

use super::task::ProofTask;

// ============================================================================
// Centralized Host Resolver
// ============================================================================

/// Centralized host resolver for prover-client
///
/// This is the single place where all host resolution logic lives:
/// - Feature flag checks for remote backends (SP1, Risc0, etc.)
/// - Backend selection logic
/// - ProofContext-based host instantiation
/// - Calls to strata_zkvm_hosts module
///
/// By implementing the PaaS `HostResolver` trait, we provide a clean interface
/// that the RemoteProofHandler can use without knowing about these details.
#[derive(Clone, Copy)]
pub(crate) struct CentralizedHostResolver;

impl HostResolver<ProofTask> for CentralizedHostResolver {
    // Type aliases for the concrete host types we use
    // RemoteHost is used for any remote proving backend (SP1, Risc0, etc.)
    #[cfg(feature = "sp1")]
    type RemoteHost = SP1Host;

    #[cfg(not(feature = "sp1"))]
    type RemoteHost = NativeHost; // Fallback type when no remote backend enabled (shouldn't be used)

    type NativeHost = NativeHost;

    fn resolve(
        &self,
        program: &ProofTask,
        backend: &ZkVmBackend,
    ) -> HostInstance<Self::RemoteHost, Self::NativeHost> {
        let proof_context = &program.0;

        match backend {
            ZkVmBackend::SP1 => {
                #[cfg(feature = "sp1")]
                {
                    // SP1 is enabled - resolve SP1 host from ProofContext

                    #[expect(clippy::absolute_paths, reason = "cfg guards are annoying")]
                    let host = strata_zkvm_hosts::sp1::get_host(proof_context);
                    HostInstance::Remote(host)
                }
                #[cfg(not(feature = "sp1"))]
                {
                    // SP1 is not enabled - this should never happen at runtime
                    // because the user shouldn't be requesting SP1 backend when
                    // the feature is disabled. Panic with a clear message.
                    let _ = proof_context; // Avoid unused variable warning
                    panic!(
                        "SP1 backend requested but sp1 feature is not enabled. \
                         Recompile with --features sp1 or use Native backend."
                    )
                }
            }

            ZkVmBackend::Risc0 => {
                let _ = proof_context; // Avoid unused variable warning
                panic!(
                    "Risc0 backend requested but risc0 feature is not enabled. \
                         Recompile with --features risc0 or use Native backend."
                )
            }

            ZkVmBackend::Native => {
                // Native is always available - resolve Native host from ProofContext
                let host = get_host(proof_context);
                HostInstance::Native(host)
            }
        }
    }
}

// ============================================================================
// Backend Selection - Single Source of Truth
// ============================================================================

/// Get the default zkVM backend based on feature flags.
///
/// This is the single source of truth for compile-time backend selection.
/// Returns `ZkVmBackend::SP1` if the `sp1` feature is enabled, otherwise `Native`.
///
/// # Example
///
/// ```ignore
/// let backend = default_backend();
/// prover_handle.submit_task(task, backend).await?;
/// ```
#[inline]
pub(crate) fn default_backend() -> ZkVmBackend {
    #[cfg(feature = "sp1")]
    {
        ZkVmBackend::SP1
    }
    #[cfg(not(feature = "sp1"))]
    {
        ZkVmBackend::Native
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_backend_selection() {
        let backend = default_backend();

        #[cfg(feature = "sp1")]
        assert!(matches!(backend, ZkVmBackend::SP1));

        #[cfg(not(feature = "sp1"))]
        assert!(matches!(backend, ZkVmBackend::Native));
    }

    #[test]
    fn test_resolver_is_copy() {
        let resolver = CentralizedHostResolver;
        let _resolver2 = resolver; // Copy
        let _resolver3 = resolver; // Copy again - should work
    }
}
