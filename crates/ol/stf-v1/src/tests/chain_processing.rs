//! Direct tests for chain-processing error branches.

use strata_identifiers::OLBlockCommitment;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types::OLBlockHeader;

use crate::{
    context::{BlockContext, BlockInfo, EpochInitialContext},
    errors::ExecError,
    process_block_start, process_epoch_initial,
    test_utils::{OLStfFixture, make_genesis_state, tamper_epoch, tamper_slot},
};

fn terminal_genesis_header() -> OLBlockHeader {
    OLStfFixture::builder()
        .execute_genesis()
        .last_completed_block()
        .header()
        .clone()
}

#[test]
fn test_epoch_initial_rejects_epoch_mismatch() {
    let mut state = make_genesis_state();
    let context = EpochInitialContext::new(1, OLBlockCommitment::null());

    let err = process_epoch_initial(&mut state, &context)
        .expect_err("invalid chain-processing input should fail");

    assert!(matches!(err, ExecError::ContextEpochMismatch(1, 0)));
    assert_eq!(state.cur_epoch(), 0);
}

#[test]
fn test_block_start_rejects_nonzero_genesis_coords() {
    let mut state = make_genesis_state();
    let block_info = BlockInfo::new(1_000_000, 1, 0);
    let context = BlockContext::new_unchecked(&block_info, None);

    let err = process_block_start(&mut state, &context)
        .expect_err("invalid chain-processing input should fail");

    assert!(matches!(err, ExecError::GenesisCoordsNonzero));
    assert_eq!(state.cur_slot(), 0);
}

#[test]
fn test_block_start_rejects_incorrect_epoch_after_parent() {
    let mut state = make_genesis_state();
    let parent_header = terminal_genesis_header();
    let block_info = BlockInfo::new(1_001_000, 1, 0);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    let err = process_block_start(&mut state, &context)
        .expect_err("invalid chain-processing input should fail");

    assert!(matches!(err, ExecError::IncorrectEpoch(0, 0, true)));
    assert_eq!(state.cur_slot(), 0);
}

#[test]
fn test_block_start_rejects_epoch_overflow_after_terminal_parent() {
    let mut state = make_genesis_state();
    let parent_header = tamper_epoch(&terminal_genesis_header(), u32::MAX);
    let block_info = BlockInfo::new(1_001_000, 1, 0);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    let err = process_block_start(&mut state, &context)
        .expect_err("invalid chain-processing input should fail");

    assert!(matches!(err, ExecError::EpochOverflow));
    assert_eq!(state.cur_epoch(), 0);
    assert_eq!(state.cur_slot(), 0);
}

#[test]
fn test_block_start_rejects_incorrect_slot_after_parent() {
    let mut state = make_genesis_state();
    let parent_header = terminal_genesis_header();
    let block_info = BlockInfo::new(1_001_000, 2, 1);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    let err = process_block_start(&mut state, &context)
        .expect_err("invalid chain-processing input should fail");

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
fn test_block_start_rejects_slot_overflow_after_parent() {
    let mut state = make_genesis_state();
    let parent_header = tamper_slot(&terminal_genesis_header(), u64::MAX);
    let block_info = BlockInfo::new(1_001_000, 0, 1);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    let err = process_block_start(&mut state, &context)
        .expect_err("invalid chain-processing input should fail");

    assert!(matches!(err, ExecError::SlotOverflow));
    assert_eq!(state.cur_epoch(), 0);
    assert_eq!(state.cur_slot(), 0);
}

#[test]
fn test_block_start_rejects_state_epoch_mismatch() {
    let mut state = make_genesis_state();
    let parent_header = terminal_genesis_header();
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    let err = process_block_start(&mut state, &context)
        .expect_err("invalid chain-processing input should fail");

    assert!(matches!(err, ExecError::HeaderEpochMismatch(1, 0)));
    assert_eq!(state.cur_slot(), 0);
}

#[test]
fn test_block_start_accepts_valid_child() {
    let fixture = OLStfFixture::builder().execute_genesis();
    let parent_header = fixture.parent_header().clone();
    let mut state = fixture.state().clone();

    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let context = BlockContext::new(&block_info, Some(&parent_header));

    process_block_start(&mut state, &context).expect("valid child block should start");

    assert_eq!(state.cur_epoch(), 1);
    assert_eq!(state.cur_slot(), 1);
}
