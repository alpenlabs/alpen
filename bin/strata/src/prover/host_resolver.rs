// TODO(STR-3064): consolidate ProverBackend/ZkVmBackend with zkaleido::ZkVm.
//! Host resolution for checkpoint proofs.
//!
//! Maps zkVM backends to the correct host instances for the checkpoint
//! proof program.

use std::sync::Arc;

use strata_config::ProverBackend;
use strata_paas::{HostInstance, HostResolver, ZkVmBackend};
use strata_proofimpl_checkpoint_new::program::CheckpointProgram;
#[cfg(feature = "sp1")]
use strata_zkvm_hosts::sp1::CHECKPOINT_NEW_HOST;
use zkaleido_native_adapter::NativeHost;

use super::task::CheckpointTask;

/// Resolves zkVM hosts for checkpoint proof generation.
///
/// Maps backends to the appropriate host:
/// - `Native` → checkpoint native host (direct execution)
/// - `SP1` → checkpoint SP1 host (remote proving via SP1 network)
#[derive(Clone, Copy)]
pub(crate) struct CheckpointHostResolver;

impl HostResolver<CheckpointTask> for CheckpointHostResolver {
    #[cfg(feature = "sp1")]
    type RemoteHost = zkaleido_sp1_host::SP1Host;

    #[cfg(not(feature = "sp1"))]
    type RemoteHost = NativeHost;

    type NativeHost = NativeHost;

    fn resolve(
        &self,
        _program: &CheckpointTask,
        backend: &ZkVmBackend,
    ) -> HostInstance<Self::RemoteHost, Self::NativeHost> {
        match backend {
            ZkVmBackend::SP1 => {
                #[cfg(feature = "sp1")]
                {
                    let host = Arc::clone(&CHECKPOINT_NEW_HOST);
                    HostInstance::Remote(host)
                }
                #[cfg(not(feature = "sp1"))]
                {
                    // validate_backend_config() rejects SP1 when the feature
                    // is disabled, so this path is unreachable in normal operation.
                    unreachable!(
                        "SP1 backend requested but sp1 feature is not enabled; \
                         validate_backend_config should have caught this at startup"
                    )
                }
            }
            ZkVmBackend::Native => {
                let host = CheckpointProgram::native_host();
                HostInstance::Native(Arc::new(host))
            }
            // backend_from_config only produces Native or SP1, so this is
            // unreachable unless a new variant is added to ZkVmBackend.
            other => unreachable!("unexpected zkVM backend: {other:?}"),
        }
    }
}

/// Maps a [`ProverBackend`] config value to the corresponding [`ZkVmBackend`].
pub(crate) fn backend_from_config(config: ProverBackend) -> ZkVmBackend {
    match config {
        ProverBackend::Native => ZkVmBackend::Native,
        ProverBackend::Sp1 => ZkVmBackend::SP1,
    }
}
