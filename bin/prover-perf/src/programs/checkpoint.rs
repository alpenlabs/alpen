//! TODO(STR-2349) Replace `build_empty_chain` with `build_chain_with_transactions` to
//! restore realistic cycle-count benchmarks. This was downgraded because the checkpoint
//! proof now requires a correct `da_state_diff_bytes` (DA witness), and computing the
//! full `OLDaPayloadV1` for a transaction-rich chain needs a `StateDiff` that encodes
//! every account balance delta, snark seqno/proof-state change, and inbox message
//! appended during the epoch. The proper fix is to either:
//!   1. Diff two `OLState` snapshots (before/after) and extract inbox messages from block bodies,
//!      or
//!   2. Route execution through `DaAccumulatingState` (requires the STF to be generic over
//!      `IStateAccessor` instead of concrete `OLState`).
//!
//! Until then, empty blocks with a slot-delta-only DA payload keep the proof correct.

use strata_bridge_params::BridgeParams;
use strata_codec::encode_to_vec;
use strata_da_framework::DaCounter;
use strata_identifiers::Buf64;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
use strata_ol_da::{GlobalStateDiff, LedgerDiff, OLDaPayloadV1, StateDiff};
use strata_ol_stf::test_utils::{build_empty_chain, make_genesis_state};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use tracing::info;
use zkaleido::{ExecutionSummary, ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram};

const SLOTS_PER_EPOCH: u64 = 9;
const NUM_BLOCKS: usize = 10;

fn prepare_input() -> CheckpointProverInput {
    let mut state = make_genesis_state();
    let mut blocks = build_empty_chain(&mut state, NUM_BLOCKS, SLOTS_PER_EPOCH)
        .expect("build_empty_chain should succeed");

    // First block is the parent (genesis); remaining blocks are the proving batch.
    let parent = blocks.remove(0).into_header();

    // Rebuild start_state: execute just the genesis block to get state after genesis.
    let mut start_state = make_genesis_state();
    let _ = build_empty_chain(&mut start_state, 1, SLOTS_PER_EPOCH)
        .expect("build_empty_chain should succeed");

    let blocks: Vec<OLBlock> = blocks
        .into_iter()
        .map(|b| {
            OLBlock::new(
                SignedOLBlockHeader::new(b.header().clone(), Buf64::zero()),
                b.body().clone(),
            )
        })
        .collect();

    // Compute DA state diff bytes: for an empty chain the only change is the slot counter.
    let terminal_header = blocks.last().expect("non-empty block list").header();
    let slot_delta = terminal_header.slot() - start_state.cur_slot();
    let slot_delta_u16 =
        u16::try_from(slot_delta).expect("slot delta exceeds u16::MAX; epoch too long");
    let da_diff = StateDiff::new(
        GlobalStateDiff::new(
            DaCounter::new_changed(slot_delta_u16),
            DaCounter::new_unchanged(),
        ),
        LedgerDiff::default(),
    );
    let da_state_diff_bytes =
        encode_to_vec(&OLDaPayloadV1::new(da_diff)).expect("encode DA payload");

    CheckpointProverInput {
        start_state: start_state.state().clone(),
        blocks,
        parent,
        da_state_diff_bytes,
        bridge_params: BridgeParams::default(),
    }
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHost) -> (String, ExecutionSummary) {
    info!("Generating execution summary for Checkpoint");
    let input = prepare_input();
    let summary =
        <CheckpointProgram as ZkVmProgram>::execute(&input, host).expect("checkpoint execution");
    (CheckpointProgram::name(), summary)
}

pub(crate) fn gen_proof(host: &impl ZkVmHost) -> (String, ProofReceiptWithMetadata) {
    info!("Generating proof for Checkpoint");
    let input = prepare_input();
    let receipt =
        <CheckpointProgram as ZkVmProgram>::prove(&input, host).expect("checkpoint proving");
    (CheckpointProgram::name(), receipt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_native_execution() {
        let input = prepare_input();
        let output = CheckpointProgram::execute(&input).unwrap();
        dbg!(output);
    }
}
