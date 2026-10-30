//! TODO(STR-2349) Replace the empty epoch with transaction-bearing blocks to
//! restore realistic cycle-count benchmarks. This was downgraded because the checkpoint
//! proof now requires a correct `da_state_diff_bytes` (DA witness), and computing the
//! full `OLDaPayloadV1` for a transaction-rich chain needs an `OLStateDiffV1` that encodes
//! every account balance delta, snark seqno/proof-state change, and inbox message
//! appended during the epoch. The proper fix is to either:
//!   1. Diff two `OLStateV1` snapshots (before/after) and extract inbox messages from block bodies,
//!      or
//!   2. Route execution through `DaAccumulatingState` (requires the STF to be generic over
//!      `IStateAccessor` instead of concrete `OLStateV1`).
//!
//! Until then, empty blocks with a slot-delta-only DA payload keep the proof correct.

use strata_codec::encode_to_vec;
use strata_da_framework::DaCounter;
use strata_identifiers::Buf64;
use strata_ol_chain_types_v1::{OLBlockV1, SignedOLBlockHeaderV1};
use strata_ol_da_types_v1::{GlobalStateDiffV1, LedgerDiffV1, OLDaPayloadV1, OLStateDiffV1};
use strata_ol_params::{OLParams, OLRuntimeParams};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_ol_state_types::{IStateAccessor, OLSpecId};
use strata_ol_stf::{execute_and_complete_block, BlockComponents, BlockContext, BlockInfo};
use strata_ol_stf_v1::test_utils::{
    build_terminal_block_components, build_terminal_genesis_components,
};
use strata_proofimpl_checkpoint::program::{CheckpointProgram, CheckpointProverInput};
use tracing::info;
use zkaleido::{ExecutionSummary, ZkVmHost, ZkVmProgram};

const SLOTS_PER_EPOCH: u64 = 9;
// Keep the fixture aligned with the spec compiled into the checkpoint guest.
const BENCHMARK_SPEC: OLSpecId = OLSpecId::V2;

fn prepare_checkpoint_input() -> CheckpointProverInput {
    // Build genesis under the benchmark spec so cycle counts exclude version takeover.
    let runtime_params = OLRuntimeParams::test_default();
    let params = OLParams::builder(runtime_params)
        .genesis_spec(BENCHMARK_SPEC)
        .build();
    let mut start_state = MemoryStateBaseLayer::new_genesis(&params)
        .expect("benchmark genesis state construction succeeds");
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let parent = execute_and_complete_block(
        BENCHMARK_SPEC,
        &mut start_state,
        BlockContext::new(&genesis_info, None),
        build_terminal_genesis_components(),
        &runtime_params,
    )
    .expect("benchmark genesis block construction succeeds")
    .into_header();

    let mut state = start_state.clone();
    let mut previous_header = parent.clone();
    let mut blocks = Vec::new();
    for slot in 1..=SLOTS_PER_EPOCH {
        let info = BlockInfo::new(previous_header.timestamp() + 1_000, slot, state.cur_epoch());
        let components = if slot == SLOTS_PER_EPOCH {
            build_terminal_block_components(state.last_l1_height() + 1)
        } else {
            BlockComponents::new_empty()
        };
        let block = execute_and_complete_block(
            BENCHMARK_SPEC,
            &mut state,
            BlockContext::new(&info, Some(&previous_header)),
            components,
            &runtime_params,
        )
        .expect("benchmark block construction succeeds");
        previous_header = block.header().clone();
        blocks.push(OLBlockV1::new(
            SignedOLBlockHeaderV1::new(previous_header.clone(), Buf64::zero()),
            block.body().clone(),
        ));
    }

    // Compute DA state diff bytes: for an empty chain the only change is the slot counter.
    let terminal_header = blocks.last().expect("non-empty block list").header();
    let slot_delta = terminal_header.slot() - start_state.cur_slot();
    let slot_delta_u16 =
        u16::try_from(slot_delta).expect("slot delta exceeds u16::MAX; epoch too long");
    let da_diff = OLStateDiffV1::new(
        GlobalStateDiffV1::new(
            DaCounter::new_changed(slot_delta_u16),
            DaCounter::new_unchanged(),
        ),
        LedgerDiffV1::default(),
    );
    let da_state_diff_bytes =
        encode_to_vec(&OLDaPayloadV1::new(da_diff)).expect("encode DA payload");

    CheckpointProverInput {
        start_state: start_state.into_container(),
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
        assert_eq!(input.start_state.cur_spec(), BENCHMARK_SPEC);
        let output =
            CheckpointProgram::execute(&input, BENCHMARK_SPEC, OLRuntimeParams::test_default())
                .unwrap();
        assert_eq!(
            *output.l2_range().start().blkid(),
            input.parent.compute_blkid()
        );
        assert_eq!(
            *output.l2_range().end().blkid(),
            input
                .blocks
                .last()
                .expect("epoch has blocks")
                .header()
                .compute_blkid()
        );
    }
}
