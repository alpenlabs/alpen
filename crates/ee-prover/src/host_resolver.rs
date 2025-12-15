use strata_paas::{HostInstance, HostResolver, ProgramType, ZkVmBackend};
use strata_primitives::proof::ProofContext;
use zkaleido_native_adapter::NativeHost;
#[cfg(feature = "sp1")]
use zkaleido_sp1_host::SP1Host;

/// Host resolver for EE account update proofs
///
/// This resolver provides host resolution specifically for `ProofContext::EthEeAcct` proofs.
/// It delegates to `strata_zkvm_hosts` for actual host instantiation.
#[derive(Clone, Copy, Debug)]
pub struct EeHostResolver;

impl<P> HostResolver<P> for EeHostResolver
where
    P: ProgramType + std::ops::Deref<Target = ProofContext>,
{
    #[cfg(feature = "sp1")]
    type RemoteHost = SP1Host;

    #[cfg(not(feature = "sp1"))]
    type RemoteHost = NativeHost;

    type NativeHost = NativeHost;

    fn resolve(
        &self,
        program: &P,
        backend: &ZkVmBackend,
    ) -> HostInstance<Self::RemoteHost, Self::NativeHost> {
        let proof_context: &ProofContext = program;

        // Ensure we're only handling EthEeAcct proofs
        match proof_context {
            ProofContext::EthEeAcct(_) => {}
            _ => {
                panic!(
                    "EeHostResolver only handles EthEeAcct proofs, got: {:?}",
                    proof_context
                );
            }
        }

        match backend {
            ZkVmBackend::SP1 => {
                #[cfg(feature = "sp1")]
                {
                    let host = strata_zkvm_hosts::sp1::get_host(proof_context);
                    HostInstance::Remote(host)
                }
                #[cfg(not(feature = "sp1"))]
                {
                    panic!(
                        "SP1 backend requested but sp1 feature is not enabled. \
                         Recompile with --features sp1 or use Native backend."
                    );
                }
            }

            ZkVmBackend::Risc0 => {
                panic!(
                    "Risc0 backend is not supported for EE account update proofs. \
                     Use SP1 or Native backend."
                );
            }

            ZkVmBackend::Native => {
                let host = strata_zkvm_hosts::native::get_host(proof_context);
                HostInstance::Native(host)
            }
        }
    }
}
