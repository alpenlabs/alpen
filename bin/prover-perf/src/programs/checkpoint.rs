use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use zkaleido::{PerformanceReport, ProofReceipt, VerifyingKey, ZkVmHostPerf, ZkVmProgramPerf};

pub(super) fn prepare_input(
    cl_stf_proof_with_vk: (ProofReceipt, VerifyingKey),
) -> CheckpointProverInput {
    let (cl_stf_proof, cl_stf_vk) = cl_stf_proof_with_vk;
    let cl_stf_proofs = vec![cl_stf_proof];
    CheckpointProverInput {
        cl_stf_proofs,
        cl_stf_vk,
    }
}

pub(crate) fn gen_perf_report(
    host: &impl ZkVmHostPerf,
    cl_stf_proof_with_vk: (ProofReceipt, VerifyingKey),
) -> PerformanceReport {
    let input = prepare_input(cl_stf_proof_with_vk);
    CheckpointProgram::perf_report(&input, host).unwrap()
}

#[cfg(test)]
mod tests {
    use strata_proofimpl_btc_blockspace::program::BtcBlockspaceProgram;
    use strata_proofimpl_cl_stf::program::ClStfProgram;
    use strata_proofimpl_evm_ee_stf::program::EvmEeProgram;

    use super::*;
    use crate::programs::cl_stf;

    #[test]
    #[ignore]
    fn test_checkpoint_native_execution() {
        let (cl_stf_proof, cl_stf_vk) = cl_stf::proof_with_vk(
            &ClStfProgram::native_host(),
            &EvmEeProgram::native_host(),
            &BtcBlockspaceProgram::native_host(),
        );
        let input = prepare_input((cl_stf_proof, cl_stf_vk));
        let output = CheckpointProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
