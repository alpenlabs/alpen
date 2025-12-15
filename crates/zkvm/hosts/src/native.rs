use std::sync::Arc;

use strata_primitives::proof::ProofContext;
use strata_proofimpl_checkpoint::program::CheckpointProgram;
use strata_proofimpl_cl_stf::program::ClStfProgram;
use strata_proofimpl_eth_ee_acct::program::EthEeAcctProgram;
use strata_proofimpl_evm_ee_stf::program::EvmEeProgram;
use zkaleido_native_adapter::NativeHost;

/// Returns a `NativeHost` instance based on the given [`ProofContext`].
///
/// NativeHost now implements `ZkVmRemoteProver` directly (executes proofs synchronously
/// and hex-encodes them in the proof ID), which combined with zkaleido's blanket impl
/// `impl<T: ZkVmHost + ZkVmRemoteProver> ZkVmRemoteHost for T {}`, automatically gives
/// it `ZkVmRemoteHost` without needing any wrappers!
pub fn get_host(id: &ProofContext) -> Arc<NativeHost> {
    let native_host = match id {
        ProofContext::EvmEeStf(..) => EvmEeProgram::native_host(),
        ProofContext::ClStf(..) => ClStfProgram::native_host(),
        ProofContext::Checkpoint(..) => CheckpointProgram::native_host(),
        ProofContext::EthEeAcct(..) => EthEeAcctProgram::native_host(),
    };
    Arc::new(native_host)
}
