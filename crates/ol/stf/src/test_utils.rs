//! Test utilities for the OL STF implementation.

use strata_acct_types::AccountId;
use strata_identifiers::{Buf32, L1BlockId};
use strata_ledger_types::{IGlobalState, IL1ViewState, StateAccessor};
use strata_ol_chain_types_new::OLBlockHeader;
use strata_ol_state_types::OLState;

use crate::{
    assembly::{execute_and_complete_block, BlockComponents, CompletedBlock},
    context::{BlockContext, BlockInfo},
    ExecResult,
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
    let genesis = execute_block(
        state,
        &genesis_info,
        None,
        BlockComponents::new_empty(),
    )?;
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
        header.epoch() as u64, expected_epoch,
        "Block epoch mismatch: expected {}, got {}",
        expected_epoch, header.epoch()
    );
    assert_eq!(
        header.slot(), expected_slot,
        "Block slot mismatch: expected {}, got {}",
        expected_slot, header.slot()
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