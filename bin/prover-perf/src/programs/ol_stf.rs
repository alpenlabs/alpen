use strata_proofimpl_ol_stf::program::{OLStfInput, OLStfProgram};
use strata_test_utils_evm_ee::L2Segment;
use strata_test_utils_l2::gen_params;
use tracing::info;
use zkaleido::{
    PerformanceReport, ProofReceiptWithMetadata, VerifyingKey, ZkVmHost, ZkVmHostPerf, ZkVmProgram,
    ZkVmProgramPerf,
};

use super::evm_ee;

pub(crate) fn prepare_input(
    evm_ee_proof_with_vk: (ProofReceiptWithMetadata, VerifyingKey),
) -> OLStfInput {
    info!("Preparing input for OL STF");
    let params = gen_params();
    let rollup_params = params.rollup().clone();

    let l2_segment = L2Segment::initialize_from_saved_evm_ee_data(1, 4);
    let chainstate = l2_segment.pre_states[0].clone();
    let (parent_block, l2_blocks) = l2_segment
        .blocks
        .split_first()
        .expect("must have at least one element");

    OLStfInput {
        rollup_params,
        chainstate,
        parent_header: parent_block.header().header().clone(),
        l2_blocks: l2_blocks.to_vec(),
        evm_ee_proof_with_vk,
    }
}

pub(crate) fn gen_perf_report(
    host: &impl ZkVmHostPerf,
    evm_ee_proof_with_vk: (ProofReceiptWithMetadata, VerifyingKey),
) -> PerformanceReport {
    info!("Generating performance report for OL STF");
    let input = prepare_input(evm_ee_proof_with_vk);
    OLStfProgram::perf_report(&input, host).unwrap()
}

pub(crate) fn gen_proof(
    host: &impl ZkVmHost,
    evm_ee_proof_with_vk: (ProofReceiptWithMetadata, VerifyingKey),
) -> ProofReceiptWithMetadata {
    info!("Generating proof for OL STF");
    let input = prepare_input(evm_ee_proof_with_vk);
    OLStfProgram::prove(&input, host).unwrap()
}

pub(crate) fn proof_with_vk(
    ol_stf_host: &impl ZkVmHost,
    evm_ee_host: &impl ZkVmHost,
) -> (ProofReceiptWithMetadata, VerifyingKey) {
    let evm_ee_proof_with_vk = evm_ee::proof_with_vk(evm_ee_host);

    let proof = gen_proof(ol_stf_host, evm_ee_proof_with_vk);
    (proof, ol_stf_host.vk())
}

#[cfg(test)]
mod tests {
    use strata_proofimpl_evm_ee_stf::program::EvmEeProgram;

    use super::*;

    #[test]
    fn test_ol_stf_native_execution() {
        let evm_ee_proof_with_vk = evm_ee::proof_with_vk(&EvmEeProgram::native_host());
        let input = prepare_input(evm_ee_proof_with_vk);
        let output = OLStfProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
