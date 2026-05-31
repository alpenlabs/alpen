use strata_identifiers::Buf64;
use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
use strata_ol_state_support_types::DaAccumulatingState;
use strata_ol_stf::{
    execute_block_batch_preseal,
    test_utils::{build_chain_with_transactions, make_genesis_state},
};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use tracing::info;
use zkaleido::{ExecutionSummary, ZkVmHost, ZkVmProgram};

const SLOTS_PER_EPOCH: u64 = 9;
const NUM_BLOCKS: usize = 10;

fn prepare_checkpoint_input() -> CheckpointProverInput {
    let mut state = make_genesis_state();
    let mut blocks = build_chain_with_transactions(&mut state, NUM_BLOCKS, SLOTS_PER_EPOCH);

    // First block is the parent (genesis); remaining blocks are the proving batch.
    let parent = blocks.remove(0).into_header();

    // Rebuild the exact pre-epoch state by executing only the shared genesis setup.
    let mut start_state = make_genesis_state();
    let _ = build_chain_with_transactions(&mut start_state, 1, SLOTS_PER_EPOCH);

    let blocks: Vec<OLBlock> = blocks
        .into_iter()
        .map(|b| {
            OLBlock::new(
                SignedOLBlockHeader::new(b.header().clone(), Buf64::zero()),
                b.body().clone(),
            )
        })
        .collect();

    // Replay the epoch through the same DA-accumulating path used by checkpoint generation.
    let mut da_state = DaAccumulatingState::new(start_state.clone());
    execute_block_batch_preseal(&mut da_state, &blocks, &parent)
        .expect("checkpoint perf replay should succeed");
    let da_state_diff_bytes = da_state
        .take_completed_epoch_da_blob()
        .expect("checkpoint perf DA finalization should succeed")
        .expect("checkpoint perf replay should produce a DA blob");

    CheckpointProverInput {
        start_state: start_state.state().clone(),
        blocks,
        parent,
        da_state_diff_bytes,
    }
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHost) -> (String, ExecutionSummary) {
    info!("Generating execution summary for Checkpoint");
    let input = prepare_checkpoint_input();
    let summary =
        <CheckpointProgram as ZkVmProgram>::execute(&input, host).expect("checkpoint execution");
    (CheckpointProgram::name(), summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_native_execution() {
        let input = prepare_checkpoint_input();
        let output = CheckpointProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
