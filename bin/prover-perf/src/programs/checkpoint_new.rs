//! TODO(STR-2349): Replace synthetic chain data with realistic test data once
//! DA verification and EE proof integration are complete. This should include real DA
//! payloads, actual snark proofs, full epoch execution, and L1 manifests.

use strata_identifiers::Buf64;
use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
use strata_ol_stf::test_utils::{build_chain_with_transactions, create_test_genesis_state};
use strata_proofimpl_checkpoint_new::program::{CheckpointProgram, CheckpointProverInput};
use tracing::info;
use zkaleido::{PerformanceReport, ZkVmHostPerf, ZkVmProgramPerf};

const SLOTS_PER_EPOCH: u64 = 9;
const NUM_BLOCKS: usize = 10;

fn prepare_checkpoint_input() -> CheckpointProverInput {
    let mut state = create_test_genesis_state();
    let mut blocks = build_chain_with_transactions(&mut state, NUM_BLOCKS, SLOTS_PER_EPOCH);

    // First block is the parent (genesis); remaining blocks are the proving batch
    let parent = blocks.remove(0).into_header();

    // Rebuild start_state: execute just the genesis block to get state after genesis
    let mut start_state = create_test_genesis_state();
    let _ = build_chain_with_transactions(&mut start_state, 1, SLOTS_PER_EPOCH);

    let blocks = blocks
        .into_iter()
        .map(|b| {
            OLBlock::new(
                SignedOLBlockHeader::new(b.header().clone(), Buf64::zero()),
                b.body().clone(),
            )
        })
        .collect();

    CheckpointProverInput {
        start_state,
        blocks,
        parent,
    }
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHostPerf) -> PerformanceReport {
    info!("Generating performance report for Checkpoint New");
    let input = prepare_checkpoint_input();
    CheckpointProgram::perf_report(&input, host).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_new_native_execution() {
        let input = prepare_checkpoint_input();
        let output = CheckpointProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
