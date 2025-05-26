use strata_proofimpl_evm_ee_stf::{primitives::EvmEeProofInput, program::EvmEeProgram};
use strata_test_utils::evm_ee::EvmSegment;
use tracing::info;
use zkaleido::{
    PerformanceReport, ProofReceipt, VerifyingKey, ZkVmHost, ZkVmHostPerf, ZkVmProgram,
    ZkVmProgramPerf,
};

pub(crate) fn prepare_input() -> EvmEeProofInput {
    info!("Preparing input for EVM EE STF");
    let segment = EvmSegment::initialize_from_saved_ee_data(1, 3);
    segment.get_inputs().clone()
}

pub(crate) fn gen_proof(host: &impl ZkVmHost) -> ProofReceipt {
    info!("Generating proof for EVM EE STF");
    let input = prepare_input();
    EvmEeProgram::prove(&input, host).unwrap()
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHostPerf) -> PerformanceReport {
    info!("Generating performance report for EVM EE STF");
    let input = prepare_input();
    EvmEeProgram::perf_report(&input, host).unwrap()
}

pub(crate) fn proof_with_vk(host: &impl ZkVmHost) -> (ProofReceipt, VerifyingKey) {
    let proof = gen_proof(host);
    let vk = host.vk();
    (proof, vk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn test_evm_ee_native_execution() {
        let input = prepare_input();
        let output = EvmEeProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
