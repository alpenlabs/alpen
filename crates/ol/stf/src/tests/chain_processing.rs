//! Direct tests for chain-processing error branches.

use strata_identifiers::OLBlockCommitment;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::OLBlockHeader;

use crate::{
    context::{BlockContext, BlockInfo, EpochInitialContext},
    errors::ExecError,
    process_block_start, process_epoch_initial,
    test_utils::{create_test_genesis_state, execute_block, genesis_block_components},
};

fn terminal_genesis_header() -> OLBlockHeader {
    let mut state = create_test_genesis_state();
    let genesis_info = BlockInfo::new_genesis(1_000_000);

    execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("terminal genesis should execute")
        .into_header()
}

#[test]
fn test_epoch_initial_rejects_epoch_mismatch() {
    let mut state = create_test_genesis_state();
    let context = EpochInitialContext::new(1, OLBlockCommitment::null());

    let err = process_epoch_initial(&mut state, &context).unwrap_err();

    assert!(matches!(err, ExecError::ChainIntegrity));
    assert_eq!(state.cur_epoch(), 0);
}

#[test]
fn test_block_start_rejects_nonzero_genesis_coords() {
    let mut state = create_test_genesis_state();
    let block_info = BlockInfo::new(1_000_000, 1, 0);
    let context = BlockContext::new_unchecked(&block_info, None);

    let err = process_block_start(&mut state, &context).unwrap_err();

    assert!(matches!(err, ExecError::GenesisCoordsNonzero));
    assert_eq!(state.cur_slot(), 0);
}

#[test]
fn test_block_start_rejects_incorrect_epoch_after_parent() {
    let mut state = create_test_genesis_state();
    let parent_header = terminal_genesis_header();
    let block_info = BlockInfo::new(1_001_000, 1, 0);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    let err = process_block_start(&mut state, &context).unwrap_err();

    assert!(matches!(err, ExecError::IncorrectEpoch(0, 0, true)));
    assert_eq!(state.cur_slot(), 0);
}

#[test]
fn test_block_start_rejects_incorrect_slot_after_parent() {
    let mut state = create_test_genesis_state();
    let parent_header = terminal_genesis_header();
    let block_info = BlockInfo::new(1_001_000, 2, 1);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    let err = process_block_start(&mut state, &context).unwrap_err();

    assert!(matches!(
        err,
        ExecError::IncorrectSlot {
            expected: 1,
            got: 2
        }
    ));
    assert_eq!(state.cur_slot(), 0);
}

#[test]
fn test_block_start_rejects_state_epoch_mismatch() {
    let mut state = create_test_genesis_state();
    let parent_header = terminal_genesis_header();
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    let err = process_block_start(&mut state, &context).unwrap_err();

    assert!(matches!(err, ExecError::EpochMismatch(1, 0)));
    assert_eq!(state.cur_slot(), 0);
}

#[test]
fn test_block_start_accepts_valid_child() {
    let parent_header = terminal_genesis_header();
    let mut state = create_test_genesis_state();
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    execute_block(&mut state, &genesis_info, None, genesis_block_components())
        .expect("terminal genesis should execute");

    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    process_block_start(&mut state, &context).expect("valid child block should start");

    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.cur_slot(), 1);
}
