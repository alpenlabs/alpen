//! Perf input for the standalone EVM EE STF SP1 guest.
//!
//! Uses the saved multi-block EVM witness fixtures so `prove` mode can exercise
//! Alpen's existing `ProofType::Compressed` path without depending on Groth16 /
//! Icicle acceleration.

use strata_proofimpl_evm_ee_stf::{primitives::EvmEeProofInput, program::EvmEeProgram};
use strata_test_utils_evm_ee::EvmSegment;
use tracing::info;
use zkaleido::{ExecutionSummary, ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram};

const START_BLOCK: u64 = 1;
const END_BLOCK: u64 = 4;

pub(super) fn prepare_input() -> EvmEeProofInput {
    info!(
        "Preparing input for EVM EE STF from saved fixture range {}..={}",
        START_BLOCK, END_BLOCK
    );
    EvmSegment::initialize_from_saved_ee_data(START_BLOCK, END_BLOCK)
        .get_inputs()
        .clone()
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHost) -> (String, ExecutionSummary) {
    info!("Generating execution summary for EVM EE STF");
    let input = prepare_input();
    let summary =
        <EvmEeProgram as ZkVmProgram>::execute(&input, host).expect("evm-ee-stf execution");
    (EvmEeProgram::name(), summary)
}

pub(crate) fn prove_with_input(
    input: &EvmEeProofInput,
    host: &impl ZkVmHost,
) -> (String, ProofReceiptWithMetadata) {
    info!("Generating proof for EVM EE STF");
    let receipt = <EvmEeProgram as ZkVmProgram>::prove(&input, host).expect("evm-ee-stf proving");
    (EvmEeProgram::name(), receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evm_ee_stf_native_execution() {
        let input = prepare_input();
        let output = EvmEeProgram::execute(&input).expect("native execution");
        assert_eq!(output.len(), input.len());
    }
}
