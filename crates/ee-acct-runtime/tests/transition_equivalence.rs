//! Tests for transition equivalence.
//!
//! This test verifies that processing N blocks as a single transition
//! produces the same final state as processing them as multiple transitions.

#![expect(unused_crate_dependencies, reason = "test dependencies")]

mod common;

use common::{
    build_chain_segment_with_multiple_blocks, build_transition_data, create_initial_state,
};
use strata_acct_types::{BitcoinAmount, SubjectId};
use strata_ee_acct_runtime::verify_and_apply_update_transition;
use strata_ee_chain_types::SubjectDepositData;
use strata_simple_ee::SimpleExecutionEnvironment;

/// Tests that processing 10 blocks as 1 transition produces the same
/// final state as processing them as 2 transitions (5 blocks each).
///
/// This test verifies an important equivalence property: breaking up
/// a sequence of blocks into multiple transitions should not affect
/// the final state.
#[test]
fn test_ten_blocks_one_vs_two_transitions() {
    let (mut initial_state, exec_state, header) = create_initial_state();
    let ee = SimpleExecutionEnvironment;

    // Create 10 deposits
    let deposits: Vec<SubjectDepositData> = (0..10)
        .map(|i| {
            let dest = SubjectId::from([(i + 1) as u8; 32]);
            let value = BitcoinAmount::from(1000u64 * (i + 1));
            SubjectDepositData::new(dest, value)
        })
        .collect();

    // Add all deposits to the initial state
    for deposit in &deposits {
        initial_state.add_tracked_balance(deposit.value());
        initial_state.add_pending_input(strata_ee_acct_types::PendingInputEntry::Deposit(
            deposit.clone(),
        ));
    }

    // Path 1: Single segment with all 10 blocks
    let (segment_all, _final_exec_state, _final_header) = build_chain_segment_with_multiple_blocks(
        ee,
        exec_state.clone(),
        header.clone(),
        deposits.clone(),
    );

    let (transition_single, shared_single, coinputs_single) = build_transition_data(
        1,
        vec![],
        vec![segment_all],
        &initial_state,
        &header,
        &exec_state,
    );

    let mut state_single = initial_state.clone();
    verify_and_apply_update_transition(
        &mut state_single,
        &transition_single,
        coinputs_single.iter().map(|v| v.as_slice()),
        &shared_single,
        &ee,
    )
    .expect("single transition should succeed");

    // Path 2: Two transitions (5 blocks each)
    // Build segment1 with deposits 0-4 (5 blocks)
    let (segment1, intermediate_exec_state, intermediate_header) =
        build_chain_segment_with_multiple_blocks(
            ee,
            exec_state.clone(),
            header.clone(),
            deposits[0..5].to_vec(),
        );

    // Transition 1: Process segment1
    let (transition1, shared1, coinputs1) = build_transition_data(
        1,
        vec![],
        vec![segment1],
        &initial_state,
        &header,
        &exec_state,
    );

    let mut state_multi = initial_state.clone();
    verify_and_apply_update_transition(
        &mut state_multi,
        &transition1,
        coinputs1.iter().map(|v| v.as_slice()),
        &shared1,
        &ee,
    )
    .expect("first transition should succeed");

    // Build segment2 with deposits 5-9 (5 blocks) starting from intermediate state
    let (segment2, _final_exec_state2, _final_header2) = build_chain_segment_with_multiple_blocks(
        ee,
        intermediate_exec_state.clone(),
        intermediate_header.clone(),
        deposits[5..10].to_vec(),
    );

    // Transition 2: Process segment2
    let (transition2, shared2, coinputs2) = build_transition_data(
        2,
        vec![],
        vec![segment2],
        &state_multi,
        &intermediate_header,
        &intermediate_exec_state,
    );

    verify_and_apply_update_transition(
        &mut state_multi,
        &transition2,
        coinputs2.iter().map(|v| v.as_slice()),
        &shared2,
        &ee,
    )
    .expect("second transition should succeed");

    // Assert final states are identical
    assert_eq!(
        state_single, state_multi,
        "Final state should be identical whether processing as 1 or 2 transitions"
    );

    // Also verify specific properties
    assert_eq!(
        state_single.tracked_balance(),
        state_multi.tracked_balance(),
        "Tracked balance should match"
    );
    assert_eq!(
        state_single.pending_inputs(),
        state_multi.pending_inputs(),
        "Pending inputs should match"
    );
}
