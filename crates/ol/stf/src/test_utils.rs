//! Test utilities for the OL STF implementation.

#![allow(unreachable_pub, reason = "test util module")]

use strata_acct_types::AccountId;
use strata_identifiers::{Buf32, L1BlockId};
use strata_ledger_types::{IGlobalState, IL1ViewState, StateAccessor};
use strata_ol_chain_types_new::OLBlockHeader;
use strata_ol_state_types::OLState;

use crate::{
    ExecResult,
    assembly::{BlockComponents, CompletedBlock, execute_and_complete_block},
    context::{BlockContext, BlockInfo},
    errors::ExecError,
    verification::{BlockExecExpectations, BlockPostStateCommitments, verify_block_classically},
};

/// Execute a block with the given block info and return the completed block.
pub fn execute_block(
    state: &mut OLState,
    block_info: &BlockInfo,
    parent_header: Option<&OLBlockHeader>,
    components: BlockComponents,
) -> ExecResult<CompletedBlock> {
    let block_context = BlockContext::new(block_info, parent_header);
    execute_and_complete_block(state, block_context, components)
}

/// Build and execute a chain of empty blocks starting from genesis.
///
/// Returns the headers of all blocks in the chain.
pub fn build_empty_chain(
    state: &mut OLState,
    num_blocks: usize,
    slots_per_epoch: u64,
) -> ExecResult<Vec<OLBlockHeader>> {
    let mut headers = Vec::with_capacity(num_blocks);

    if num_blocks == 0 {
        return Ok(headers);
    }

    // Execute genesis block
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis = execute_block(state, &genesis_info, None, BlockComponents::new_empty())?;
    headers.push(genesis.header().clone());

    // Execute subsequent blocks
    for i in 1..num_blocks {
        let slot = i as u64;
        let epoch = slot / slots_per_epoch;
        let parent = &headers[i - 1];
        let timestamp = 1000000 + (i as u64 * 1000);
        let block_info = BlockInfo::new(timestamp, slot, epoch as u32);

        let block = execute_block(
            state,
            &block_info,
            Some(parent),
            BlockComponents::new_empty(),
        )?;
        headers.push(block.header().clone());
    }

    Ok(headers)
}

/// Create test account IDs with predictable values.
pub fn test_account_id(index: u32) -> AccountId {
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&index.to_le_bytes());
    AccountId::from(bytes)
}

/// Create a test L1 block ID with predictable values.
pub fn test_l1_block_id(index: u32) -> L1BlockId {
    let mut bytes = [0u8; 32];
    bytes[0..4].copy_from_slice(&index.to_le_bytes());
    L1BlockId::from(Buf32::from(bytes))
}

/// Assert that a block header matches expected epoch and slot values.
pub fn assert_block_position(header: &OLBlockHeader, expected_epoch: u64, expected_slot: u64) {
    assert_eq!(
        header.epoch() as u64,
        expected_epoch,
        "Block epoch mismatch: expected {}, got {}",
        expected_epoch,
        header.epoch()
    );
    assert_eq!(
        header.slot(),
        expected_slot,
        "Block slot mismatch: expected {}, got {}",
        expected_slot,
        header.slot()
    );
}

/// Assert that the state has been properly updated after block execution.
pub fn assert_state_updated(state: &mut OLState, expected_epoch: u64, expected_slot: u64) {
    assert_eq!(
        state.l1_view().cur_epoch() as u64,
        expected_epoch,
        "State epoch mismatch"
    );
    assert_eq!(
        state.global_mut().cur_slot(),
        expected_slot,
        "State slot mismatch"
    );
}

// ===== Verification Test Utilities =====

/// Create BlockExecExpectations from a CompletedBlock.
/// This is used to verify that a block can be verified after assembly.
pub fn create_expectations_from_block(block: &CompletedBlock) -> BlockExecExpectations {
    let post_state_roots = if let Some(l1_update) = block.body().l1_update() {
        // Terminal block has both preseal and final state roots
        BlockPostStateCommitments::Terminal(
            l1_update.preseal_state_root().clone(),
            block.header().state_root().clone(),
        )
    } else {
        // Non-terminal block has only final state root
        BlockPostStateCommitments::Common(block.header().state_root().clone())
    };

    BlockExecExpectations::new(post_state_roots, block.header().logs_root().clone())
}

/// Verify a block using verify_block_classically and assert it succeeds.
pub fn verify_block<S: StateAccessor>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
    exp: &BlockExecExpectations,
) -> ExecResult<()> {
    verify_block_classically(state, header, parent_header, body, exp)
}

/// Assert that block verification succeeds.
pub fn assert_verification_succeeds<S: StateAccessor>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
    exp: &BlockExecExpectations,
) {
    let result = verify_block(state, header, parent_header, body, exp);
    assert!(
        result.is_ok(),
        "Block verification failed when it should have succeeded: {:?}",
        result.err()
    );
}

/// Assert that block verification fails with a specific error.
pub fn assert_verification_fails_with(
    state: &mut impl StateAccessor,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
    exp: &BlockExecExpectations,
    error_matcher: impl Fn(&ExecError) -> bool,
) {
    let result = verify_block(state, header, parent_header, body, exp);
    assert!(
        result.is_err(),
        "Block verification succeeded when it should have failed"
    );

    let err = result.unwrap_err();
    assert!(error_matcher(&err), "Unexpected error type. Got: {:?}", err);
}

/// Create a tampered block header with a different parent block ID.
pub fn tamper_parent_blkid(
    header: &OLBlockHeader,
    new_parent: strata_ol_chain_types_new::OLBlockId,
) -> OLBlockHeader {
    let mut tampered = header.clone();
    // We need to create a new header with the modified parent
    OLBlockHeader::new(
        tampered.timestamp(),
        tampered.slot(),
        tampered.epoch(),
        new_parent,
        tampered.body_root().clone(),
        tampered.state_root().clone(),
        tampered.logs_root().clone(),
    )
}

/// Create a tampered block header with a different state root.
pub fn tamper_state_root(header: &OLBlockHeader, new_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.slot(),
        header.epoch(),
        header.parent_blkid().clone(),
        header.body_root().clone(),
        new_root,
        header.logs_root().clone(),
    )
}

/// Create a tampered block header with a different logs root.
pub fn tamper_logs_root(header: &OLBlockHeader, new_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.slot(),
        header.epoch(),
        header.parent_blkid().clone(),
        header.body_root().clone(),
        header.state_root().clone(),
        new_root,
    )
}

/// Create a tampered block header with a different body root.
pub fn tamper_body_root(header: &OLBlockHeader, new_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.slot(),
        header.epoch(),
        header.parent_blkid().clone(),
        new_root,
        header.state_root().clone(),
        header.logs_root().clone(),
    )
}

/// Create a tampered block header with a different slot.
pub fn tamper_slot(header: &OLBlockHeader, new_slot: u64) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        new_slot,
        header.epoch(),
        header.parent_blkid().clone(),
        header.body_root().clone(),
        header.state_root().clone(),
        header.logs_root().clone(),
    )
}

/// Create a tampered block header with a different epoch.
pub fn tamper_epoch(header: &OLBlockHeader, new_epoch: u32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.slot(),
        new_epoch,
        header.parent_blkid().clone(),
        header.body_root().clone(),
        header.state_root().clone(),
        header.logs_root().clone(),
    )
}
