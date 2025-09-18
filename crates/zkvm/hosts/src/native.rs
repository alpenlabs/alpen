use strata_primitives::proof::ProofContext;
use strata_proofimpl_checkpoint::program::CheckpointProgram;
use strata_proofimpl_cl_stf::program::ClStfProgram;
use strata_proofimpl_evm_ee_stf::program::EvmEeProgram;
use zkaleido_native_adapter::NativeHost;

/// Returns a reference to the appropriate [`NativeHost`] instance based on the given
/// [`ProofContext`].
///
/// This function maps the `ProofContext` variant to its corresponding [`NativeHost`] instance,
/// allowing for efficient host selection for different proof types.
pub fn get_host(id: &ProofContext) -> NativeHost {
    match id {
        ProofContext::EvmEeStf(..) => EvmEeProgram::native_host(),
        ProofContext::ClStf(..) => ClStfProgram::native_host(),
        ProofContext::Checkpoint(..) => CheckpointProgram::native_host(),
    }
}
