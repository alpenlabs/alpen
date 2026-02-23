//! Test utilities for building realistic [`CheckpointProverInput`] with diverse OL transactions.
//!
//! TODO(STR-2349): Replace synthetic chain data with realistic test data once STR-1366
//! (DA verification) and EE proof integration are complete. This should include real DA
//! payloads, actual snark proofs, full epoch execution, and L1 manifests.

use std::mem;

use strata_acct_types::{BitcoinAmount, MsgPayload};
use strata_asm_common::AsmManifest;
use strata_identifiers::{Buf32, Buf64, L1BlockId, WtxidsRoot};
use strata_ledger_types::{AccountTypeState, IStateAccessor, NewAccountData};
use strata_ol_chain_types_new::{
    GamTxPayload, OLBlock, OLL1ManifestContainer, OLTransaction, SignedOLBlockHeader,
    TransactionAttachment, TransactionPayload,
};
use strata_ol_state_types::{OLSnarkAccountState, OLState};
use strata_ol_stf::{
    BlockComponents, BlockInfo, CompletedBlock, SEQUENCER_ACCT_ID,
    test_utils::{
        InboxMmrTracker, SnarkUpdateBuilder, create_empty_account, create_test_genesis_state,
        execute_block, get_snark_state_expect, get_test_recipient_account_id,
        get_test_snark_account_id, get_test_state_root, test_account_id,
    },
};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::MessageEntry;

use crate::program::CheckpointProverInput;

const SLOTS_PER_EPOCH: u64 = 9;
const NUM_BLOCKS: usize = 10;
const SNARK_INITIAL_BALANCE: u64 = 100_000_000;
const TRANSFER_AMOUNT: u64 = 1_000_000;

fn create_snark_account(state: &mut OLState) {
    let snark_id = get_test_snark_account_id();
    let update_vk = PredicateKey::always_accept();
    let initial_state_root = get_test_state_root(1);
    let snark_state = OLSnarkAccountState::new_fresh(update_vk, initial_state_root);
    let balance = BitcoinAmount::from_sat(SNARK_INITIAL_BALANCE);
    let new_acct_data = NewAccountData::new(balance, AccountTypeState::Snark(snark_state));
    state
        .create_new_account(snark_id, new_acct_data)
        .expect("should create snark account");
}

/// Builds a chain of blocks with a mix of transaction types.
///
/// Uses a 4-block cycle after genesis:
/// - `i % 4 == 1`: GAM to snark account (populates inbox for later processing)
/// - `i % 4 == 2`: GAM to regular target
/// - `i % 4 == 3`: Complex SnarkAccountUpdate (processes inbox messages with MMR proofs, includes
///   output transfers)
/// - `i % 4 == 0`: Empty block
fn build_chain_with_transactions(
    state: &mut OLState,
    num_blocks: usize,
    slots_per_epoch: u64,
) -> Vec<CompletedBlock> {
    let mut blocks = Vec::with_capacity(num_blocks);

    let gam_target = test_account_id(1);
    let snark_id = get_test_snark_account_id();
    let recipient_id = get_test_recipient_account_id();

    // Create accounts before genesis
    create_snark_account(state);
    create_empty_account(state, gam_target);
    create_empty_account(state, recipient_id);

    // Terminal genesis (with manifest) so epoch advances from 0 to 1
    let genesis_manifest = AsmManifest::new(
        0,
        L1BlockId::from(Buf32::from([0u8; 32])),
        WtxidsRoot::from(Buf32::from([0u8; 32])),
        vec![],
    );
    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);
    let genesis =
        execute_block(state, &genesis_info, None, genesis_components).expect("genesis should work");
    blocks.push(genesis);

    let mut state_root_counter: u8 = 2;
    let mut inbox_tracker = InboxMmrTracker::new();
    let mut pending_msgs: Vec<MessageEntry> = Vec::new();
    let mut pending_proofs = Vec::new();

    for i in 1..num_blocks {
        let slot = i as u64;
        let epoch = ((slot - 1) / slots_per_epoch + 1) as u32;
        let parent = blocks[i - 1].header();
        let timestamp = 1_000_000 + (i as u64 * 1000);
        let block_info = BlockInfo::new(timestamp, slot, epoch);

        let is_terminal = slot.is_multiple_of(slots_per_epoch);

        let components = if is_terminal {
            let dummy_manifest = AsmManifest::new(
                0,
                L1BlockId::from(Buf32::from([0u8; 32])),
                WtxidsRoot::from(Buf32::from([0u8; 32])),
                vec![],
            );
            let tx = TransactionPayload::GenericAccountMessage(
                GamTxPayload::new(gam_target, format!("terminal block {i}").into_bytes())
                    .expect("GamTxPayload creation should succeed"),
            );
            BlockComponents::new(
                strata_ol_chain_types_new::OLTxSegment::new(vec![OLTransaction::new(
                    tx,
                    TransactionAttachment::default(),
                )])
                .expect("tx segment should be within limits"),
                Some(
                    OLL1ManifestContainer::new(vec![dummy_manifest])
                        .expect("single manifest should succeed"),
                ),
            )
        } else if i % 4 == 1 {
            // GAM to snark account: populates the snark's inbox for later processing
            let msg_data = format!("inbox msg at slot {i}").into_bytes();
            let tx = TransactionPayload::GenericAccountMessage(
                GamTxPayload::new(snark_id, msg_data.clone())
                    .expect("GamTxPayload creation should succeed"),
            );

            let msg_entry = MessageEntry::new(
                SEQUENCER_ACCT_ID,
                epoch,
                MsgPayload::new(BitcoinAmount::from_sat(0), msg_data),
            );
            let proof = inbox_tracker.add_message(&msg_entry);
            pending_msgs.push(msg_entry);
            pending_proofs.push(proof);

            BlockComponents::new_txs(vec![tx])
        } else if i % 4 == 3 && !pending_msgs.is_empty() {
            // Complex SnarkAccountUpdate: processes inbox messages with valid MMR proofs
            // and transfers funds to the recipient account
            let (_, snark_state) = get_snark_state_expect(state, snark_id);
            let builder = SnarkUpdateBuilder::from_snark_state(snark_state.clone())
                .with_processed_msgs(mem::take(&mut pending_msgs))
                .with_inbox_proofs(mem::take(&mut pending_proofs))
                .with_transfer(recipient_id, TRANSFER_AMOUNT);
            let new_state_root = get_test_state_root(state_root_counter);
            state_root_counter = state_root_counter.wrapping_add(1);
            let tx = builder.build(snark_id, new_state_root, vec![0u8; 32]);
            BlockComponents::new_txs(vec![tx])
        } else if i % 4 == 2 {
            // GAM to regular target account
            let tx = TransactionPayload::GenericAccountMessage(
                GamTxPayload::new(gam_target, format!("message at slot {i}").into_bytes())
                    .expect("GamTxPayload creation should succeed"),
            );
            BlockComponents::new_txs(vec![tx])
        } else {
            BlockComponents::new_empty()
        };

        let block = execute_block(state, &block_info, Some(parent), components)
            .expect("block execution should succeed");
        blocks.push(block);
    }

    blocks
}

/// Prepares a [`CheckpointProverInput`] with a realistic mix of OL transactions.
///
/// The generated chain includes GAM transactions, complex SnarkAccountUpdates
/// with inbox message processing and output transfers, and empty blocks.
pub fn prepare_checkpoint_input() -> CheckpointProverInput {
    let mut state = create_test_genesis_state();
    let mut blocks = build_chain_with_transactions(&mut state, NUM_BLOCKS, SLOTS_PER_EPOCH);

    // First block is the parent (genesis); remaining blocks are the proving batch
    let parent = blocks.remove(0).into_header();

    // Rebuild start_state: execute just the genesis block to get state after genesis
    let mut start_state = create_test_genesis_state();
    let _ = build_chain_with_transactions(&mut start_state, 1, SLOTS_PER_EPOCH);

    let blocks = blocks
        .into_iter()
        .map(|b| {
            OLBlock::new(
                SignedOLBlockHeader::new(b.header().clone(), Buf64::zero()),
                b.body().clone(),
            )
        })
        .collect();

    CheckpointProverInput {
        start_state,
        blocks,
        parent,
    }
}
