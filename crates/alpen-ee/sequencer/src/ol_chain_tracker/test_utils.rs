//! Shared test utilities for ol_chain_tracker tests.

use alpen_ee_common::{ExecBlockRecord, OLBlockData, OLChainStatus};
use strata_acct_types::{AccountId, BitcoinAmount, Hash, MsgPayload};
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::{BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage};
use strata_identifiers::{Buf32, EpochCommitment, OLBlockCommitment, OLBlockId};
use strata_snark_acct_types::MessageEntry;

/// Helper to create a block commitment with a given slot.
/// The slot is encoded into the blkid for uniqueness.
pub(crate) fn make_block(slot: u64) -> OLBlockCommitment {
    let mut blkid_bytes = [0u8; 32];
    blkid_bytes[0..8].copy_from_slice(&slot.to_le_bytes());
    OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from(blkid_bytes)))
}

/// Helper to create a block commitment with a given slot and specific id byte.
/// Useful for creating blocks at the same slot with different IDs (reorg scenarios).
pub(crate) fn make_block_with_id(slot: u64, id_byte: u8) -> OLBlockCommitment {
    let mut blkid_bytes = [id_byte; 32];
    blkid_bytes[0..8].copy_from_slice(&slot.to_le_bytes());
    OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from(blkid_bytes)))
}

/// Helper to create a dummy message entry with a given satoshi value.
pub(crate) fn make_message(value: u64) -> MessageEntry {
    MessageEntry::new(
        AccountId::new([0u8; 32]),
        0,
        MsgPayload::new(BitcoinAmount::from_sat(value), vec![]),
    )
}

/// Helper to create an EpochCommitment from an OLBlockCommitment.
pub(crate) fn make_epoch_from_block(epoch: u32, block: OLBlockCommitment) -> EpochCommitment {
    EpochCommitment::new(epoch, block.slot(), *block.blkid())
}

/// Helper to create OLChainStatus with the given finalized block.
pub(crate) fn make_chain_status(finalized: OLBlockCommitment) -> OLChainStatus {
    let epoch = make_epoch_from_block(0, finalized);
    OLChainStatus {
        latest: finalized,
        confirmed: epoch,
        finalized: epoch,
    }
}

/// Helper to create OLBlockData for a block with messages.
pub(crate) fn make_block_data(
    block: OLBlockCommitment,
    messages: Vec<MessageEntry>,
    next_inbox_msg_idx: u64,
) -> OLBlockData {
    OLBlockData {
        commitment: block,
        inbox_messages: messages,
        next_inbox_msg_idx,
    }
}

/// Creates a chain of OL blocks starting from base_slot.
///
/// Returns blocks with slots [base_slot, base_slot+1, ..., base_slot+count-1]
/// Each block has a unique ID derived from its slot.
pub(crate) fn create_ol_block_chain(base_slot: u64, count: usize) -> Vec<OLBlockCommitment> {
    (0..count)
        .map(|i| {
            let slot = base_slot + i as u64;
            make_block(slot)
        })
        .collect()
}

/// Creates OLBlockData for each block in the chain.
/// Each block gets one message with value = slot * 100.
pub(crate) fn create_block_data_chain(
    blocks: &[OLBlockCommitment],
    next_inbox_msg_idx: u64,
) -> Vec<OLBlockData> {
    blocks
        .iter()
        .enumerate()
        .map(|(idx, block)| {
            let msg = make_message(block.slot() * 100);
            make_block_data(*block, vec![msg], next_inbox_msg_idx + idx as u64 + 1)
        })
        .collect()
}

/// Creates a mock ExecBlockRecord that references the given OL block.
pub(crate) fn create_mock_exec_record(ol_block: OLBlockCommitment) -> ExecBlockRecord {
    let hash_bytes = [ol_block.slot() as u8; 32];
    let hash = Hash::from(Buf32::new(hash_bytes));

    let package = ExecBlockPackage::new(
        ExecBlockCommitment::new(hash, hash),
        BlockInputs::new_empty(),
        BlockOutputs::new_empty(),
    );

    let account_state = EeAccountState::new(hash, BitcoinAmount::ZERO, vec![], vec![]);

    ExecBlockRecord::new(
        package,
        account_state,
        ol_block.slot(),
        ol_block,
        1_000_000,
        Hash::default(),
        0,
        vec![],
    )
}
