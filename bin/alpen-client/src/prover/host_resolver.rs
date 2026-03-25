//! Host resolution for EE proof tasks.
//!
//! Stub implementation pending PR #1522 which introduces `EeChunkProgram`
//! and `EeAcctProgram` with their SP1 guests and native hosts.

use alpen_ee_common::EeProofTask;
use strata_paas::{HostInstance, HostResolver, ZkVmBackend};
use zkaleido_native_adapter::NativeHost;

/// Resolves zkVM hosts for EE proof tasks.
///
/// TODO(PR #1522): Add SP1 host support once guest ELFs are available.
#[derive(Clone, Copy)]
pub(crate) struct EeHostResolver;

impl HostResolver<EeProofTask> for EeHostResolver {
    // NativeHost as fallback RemoteHost type (same pattern as prover-client).
    // Will be replaced with SP1Host behind feature flag once PR #1522 lands.
    type RemoteHost = NativeHost;
    type NativeHost = NativeHost;

    fn resolve(
        &self,
        _program: &EeProofTask,
        backend: &ZkVmBackend,
    ) -> HostInstance<Self::RemoteHost, Self::NativeHost> {
        match backend {
            ZkVmBackend::Native => {
                // TODO(PR #1522): Create proper native hosts via
                // EeChunkProgram::native_host() / EeAcctProgram::native_host()
                todo!("native host resolution blocked on PR #1522 (guest programs)")
            }
            ZkVmBackend::SP1 => {
                // TODO(PR #1522): Load SP1 guest ELFs, create SP1Host instances.
                todo!("SP1 host resolution blocked on PR #1522 (guest ELFs)")
            }
            ZkVmBackend::Risc0 => {
                panic!("Risc0 backend not supported for EE proofs")
            }
        }
    }
}

/// Get the default zkVM backend for EE proofs.
#[inline]
pub(crate) fn default_ee_backend() -> ZkVmBackend {
    // TODO: support SP1 behind feature flag once PR #1522 lands.
    ZkVmBackend::Native
}
