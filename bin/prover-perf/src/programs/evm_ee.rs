use strata_proofimpl_evm_ee_stf::{primitives::EvmEeProofInput, program::EvmEeProgram};
use strata_test_utils_evm_ee::EvmSegment;
use tracing::info;
use zkaleido::{
    PerformanceReport, ProofReceiptWithMetadata, VerifyingKey, ZkVmHost, ZkVmHostPerf, ZkVmProgram,
    ZkVmProgramPerf,
};

pub(crate) fn prepare_input() -> EvmEeProofInput {
    info!("Preparing input for EVM EE STF");
    let segment = EvmSegment::initialize_from_saved_ee_data(2, 4);
    segment.get_inputs().clone()
}

pub(crate) fn gen_proof(host: &impl ZkVmHost) -> ProofReceiptWithMetadata {
    info!("Generating proof for EVM EE STF");
    let input = prepare_input();
    EvmEeProgram::prove(&input, host).unwrap()
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHostPerf) -> PerformanceReport {
    info!("Generating performance report for EVM EE STF");
    let input = prepare_input();
    EvmEeProgram::perf_report(&input, host).unwrap()
}

pub(crate) fn proof_with_vk(host: &impl ZkVmHost) -> (ProofReceiptWithMetadata, VerifyingKey) {
    let proof = gen_proof(host);
    let vk = host.vk();
    (proof, vk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evm_ee_native_execution() {
        let input = prepare_input();
        let output = EvmEeProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
