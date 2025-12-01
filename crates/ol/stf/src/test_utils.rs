//! Test utilities for the OL STF implementation.

#![allow(unreachable_pub, reason = "test util module")]

use strata_acct_types::AccountId;
use strata_asm_common::AsmManifest;
use strata_identifiers::{Buf32, L1BlockId, WtxidsRoot};
use strata_ledger_types::{IGlobalState, IL1ViewState, StateAccessor};
use strata_ol_chain_types_new::OLBlockHeader;
use strata_ol_state_types::OLState;

use crate::{
    ExecResult,
    assembly::{
        BlockComponents, CompletedBlock, ConstructBlockOutput, construct_block,
        execute_and_complete_block,
    },
    context::{BlockContext, BlockInfo},
    errors::ExecError,
    verification::verify_block,
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

/// Execute a block and return the construct output which includes both the completed block and
/// execution outputs. This is useful for tests that need to inspect the logs.
pub fn execute_block_with_outputs(
    state: &mut OLState,
    block_info: &BlockInfo,
    parent_header: Option<&OLBlockHeader>,
    components: BlockComponents,
) -> ExecResult<ConstructBlockOutput> {
    let block_context = BlockContext::new(block_info, parent_header);
    construct_block(state, block_context, components)
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

    // Execute genesis block (always terminal)
    let genesis_info = BlockInfo::new_genesis(1000000);
    let genesis_manifest = AsmManifest::new(
        L1BlockId::from(Buf32::from([0u8; 32])),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![],
    );
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);
    let genesis = execute_block(state, &genesis_info, None, genesis_components)?;
    headers.push(genesis.header().clone());

    // Execute subsequent blocks
    for i in 1..num_blocks {
        let slot = i as u64;
        // With genesis as terminal: epoch 0 is just genesis, then normal epochs
        let epoch = ((slot - 1) / slots_per_epoch + 1) as u32;
        let parent = &headers[i - 1];
        let timestamp = 1000000 + (i as u64 * 1000);
        let block_info = BlockInfo::new(timestamp, slot, epoch);

        // Check if this should be a terminal block
        // After genesis, terminal blocks are at slots that are multiples of slots_per_epoch
        let is_terminal = slot.is_multiple_of(slots_per_epoch);

        let components = if is_terminal {
            // Create a terminal block with a dummy manifest
            let dummy_manifest = AsmManifest::new(
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([0u8; 32])),
                vec![],
            );
            BlockComponents::new_manifests(vec![dummy_manifest])
        } else {
            BlockComponents::new_empty()
        };

        let block = execute_block(state, &block_info, Some(parent), components)?;
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
        "test: state epoch mismatch"
    );
    assert_eq!(
        state.global().cur_slot(),
        expected_slot,
        "test: state slot mismatch"
    );
}

// ===== Verification Test Utilities =====

/// Assert that block verification succeeds.
pub fn assert_verification_succeeds<S: StateAccessor>(
    state: &mut S,
    header: &OLBlockHeader,
    parent_header: Option<OLBlockHeader>,
    body: &strata_ol_chain_types_new::OLBlockBody,
) {
    let result = verify_block(state, header, parent_header, body);
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
    error_matcher: impl Fn(&ExecError) -> bool,
) {
    let result = verify_block(state, header, parent_header, body);
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
    // We need to create a new header with the modified parent
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        header.epoch(),
        new_parent,
        *header.body_root(),
        *header.state_root(),
        *header.logs_root(),
    )
}

/// Create a tampered block header with a different state root.
pub fn tamper_state_root(header: &OLBlockHeader, new_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        header.epoch(),
        *header.parent_blkid(),
        *header.body_root(),
        new_root,
        *header.logs_root(),
    )
}

/// Create a tampered block header with a different logs root.
pub fn tamper_logs_root(header: &OLBlockHeader, new_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        header.epoch(),
        *header.parent_blkid(),
        *header.body_root(),
        *header.state_root(),
        new_root,
    )
}

/// Create a tampered block header with a different body root.
pub fn tamper_body_root(header: &OLBlockHeader, new_root: Buf32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        header.epoch(),
        *header.parent_blkid(),
        new_root,
        *header.state_root(),
        *header.logs_root(),
    )
}

/// Create a tampered block header with a different slot.
pub fn tamper_slot(header: &OLBlockHeader, new_slot: u64) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        new_slot,
        header.epoch(),
        *header.parent_blkid(),
        *header.body_root(),
        *header.state_root(),
        *header.logs_root(),
    )
}

/// Create a tampered block header with a different epoch.
pub fn tamper_epoch(header: &OLBlockHeader, new_epoch: u32) -> OLBlockHeader {
    OLBlockHeader::new(
        header.timestamp(),
        header.flags(),
        header.slot(),
        new_epoch,
        *header.parent_blkid(),
        *header.body_root(),
        *header.state_root(),
        *header.logs_root(),
    )
}
