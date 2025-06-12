use strata_l1tx::filter::types::TxFilterConfig;
use strata_proofimpl_btc_blockspace::{logic::BlockScanProofInput, program::BtcBlockspaceProgram};
use strata_test_utils::{bitcoin_mainnet_segment::BtcChainSegment, l2::gen_params};
use tracing::info;
use zkaleido::{
    PerformanceReport, ProofReceipt, VerifyingKey, ZkVmHost, ZkVmHostPerf, ZkVmProgram,
    ZkVmProgramPerf,
};

fn prepare_input() -> BlockScanProofInput {
    info!("Preparing input for BTC Blockcan");
    let params = gen_params();
    let rollup_params = params.rollup();
    let tx_filters = TxFilterConfig::derive_from(rollup_params).unwrap();

    let btc_blocks = BtcChainSegment::load()
        .get_blocks(rollup_params.genesis_l1_height + 1, 3)
        .expect("failed to get blocks");
    BlockScanProofInput {
        btc_blocks,
        tx_filters,
    }
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHostPerf) -> PerformanceReport {
    info!("Generating performance report for BTC Blockcan");
    let input = prepare_input();
    BtcBlockspaceProgram::perf_report(&input, host).unwrap()
}

pub(crate) fn gen_proof(host: &impl ZkVmHost) -> ProofReceipt {
    info!("Generating proof for BTC Blockscan");
    let input = prepare_input();
    BtcBlockspaceProgram::prove(&input, host).unwrap()
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
    fn test_btc_blockscan_native_execution() {
        let input = prepare_input();
        let output = BtcBlockspaceProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
