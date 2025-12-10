use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use zkaleido::{
    PerformanceReport, ProofReceiptWithMetadata, VerifyingKey, ZkVmHostPerf, ZkVmProgramPerf,
};

pub(super) fn prepare_input(
    ol_stf_proof_with_vk: (ProofReceiptWithMetadata, VerifyingKey),
) -> CheckpointProverInput {
    let (ol_stf_proof, ol_stf_vk) = ol_stf_proof_with_vk;
    let ol_stf_proofs = vec![ol_stf_proof];
    CheckpointProverInput {
        ol_stf_proofs,
        ol_stf_vk,
    }
}

pub(crate) fn gen_perf_report(
    host: &impl ZkVmHostPerf,
    ol_stf_proof_with_vk: (ProofReceiptWithMetadata, VerifyingKey),
) -> PerformanceReport {
    let input = prepare_input(ol_stf_proof_with_vk);
    CheckpointProgram::perf_report(&input, host).unwrap()
}

#[cfg(test)]
mod tests {
    use strata_proofimpl_evm_ee_stf::program::EvmEeProgram;
    use strata_proofimpl_ol_stf::program::OLStfProgram;

    use super::*;
    use crate::programs::ol_stf;

    #[test]
    fn test_checkpoint_native_execution() {
        let (ol_stf_proof, ol_stf_vk) =
            ol_stf::proof_with_vk(&OLStfProgram::native_host(), &EvmEeProgram::native_host());
        let input = prepare_input((ol_stf_proof, ol_stf_vk));
        let output = CheckpointProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
