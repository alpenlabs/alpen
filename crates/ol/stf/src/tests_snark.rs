//! Tests for snark account operations including verification and state transitions

use ssz::Encode as _;
use ssz_primitives::FixedBytes;
use strata_acct_types::{AccountId, AcctError, BitcoinAmount, Hash, MsgPayload, RawMerkleProof};
use strata_asm_common::{AsmLogEntry, AsmManifest, logging::debug};
use strata_asm_manifest_types::DepositIntentLogData;
use strata_identifiers::{AccountSerial, Buf32, Epoch, L1BlockId, Slot, SubjectId, WtxidsRoot};
use strata_ledger_types::*;
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_chain_types_new::{
    GamTxPayload, SimpleWithdrawalIntentLogData, SnarkAccountUpdateLogData,
    SnarkAccountUpdateTxPayload, TransactionPayload,
};
use strata_ol_msg_types::{WITHDRAWAL_MSG_TYPE_ID, WithdrawalMsgData};
use strata_ol_state_types::{OLAccountState, OLSnarkAccountState, OLState};
use strata_predicate::PredicateKey;
use strata_snark_acct_types::{
    AccumulatorClaim, LedgerRefProofs, LedgerRefs, MessageEntry, MessageEntryProof, MmrEntryProof,
    OutputMessage, OutputTransfer, ProofState, SnarkAccountUpdate, SnarkAccountUpdateContainer,
    UpdateAccumulatorProofs, UpdateOperationData, UpdateOutputs,
};

use crate::{
    CompletedBlock, SEQUENCER_ACCT_ID,
    assembly::BlockComponents,
    constants::{BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL},
    context::BlockInfo,
    errors::ExecError,
    test_utils::{
        InboxMmrTracker, SnarkUpdateBuilder, TEST_NONEXISTENT_ID, TEST_RECIPIENT_ID,
        TEST_SNARK_ACCOUNT_ID, create_empty_account, execute_block, execute_block_with_outputs,
        execute_tx_in_block, get_test_recipient_account_id, get_test_snark_account_id,
        get_test_state_root, setup_genesis_with_snark_account, test_account_id, test_l1_block_id,
    },
    verification::*,
};

// === Shared Test Helpers ===

/// Helper to track inbox MMR proofs in parallel with the actual STF inbox MMR.
/// This allows generating valid MMR proofs for testing by maintaining proofs as leaves are added.
struct InboxMmrTracker {
    mmr: Mmr64,
    proofs: Vec<MerkleProof<[u8; 32]>>,
}

impl InboxMmrTracker {
    fn new() -> Self {
        Self {
            mmr: Mmr64::from_generic(&CompactMmr64::new(64)),
            proofs: Vec::new(),
        }
    }

    /// Adds a message entry to the tracker and returns a proof for it.
    /// Uses TreeHash for consistent hashing with insertion and verification.
    fn add_message(&mut self, entry: &MessageEntry) -> MessageEntryProof {
        // Compute hash using TreeHash, matching both insertion and verification
        let hash = <MessageEntry as TreeHash>::tree_hash_root(entry);

        // Add to MMR with proof tracking
        let proof = Mmr::<StrataHasher>::add_leaf_updating_proof_list(
            &mut self.mmr,
            hash.into_inner(),
            &mut self.proofs,
        )
        .expect("mmr: can't add leaf");

        self.proofs.push(proof.clone());

        // Convert MerkleProof to RawMerkleProof (strip the index)
        let raw_proof = RawMerkleProof {
            cohashes: proof
                .cohashes()
                .iter()
                .map(|h| FixedBytes::from(*h))
                .collect::<Vec<_>>()
                .into(),
        };

        MessageEntryProof::new(entry.clone(), raw_proof)
    }

    /// Returns the number of entries in the tracked MMR
    fn num_entries(&self) -> u64 {
        self.mmr.num_entries()
    }
}

/// Creates a SNARK account with initial balance and executes an empty genesis block
/// Returns (genesis_header, account_serial)
/// The inbox will be empty - no deposit messages
fn setup_genesis_with_snark_account(
    state: &mut OLState,
    snark_id: AccountId,
    initial_balance: u64,
) -> (strata_ol_chain_types_new::OLBlockHeader, AccountSerial) {
    // Create snark account with initial balance directly
    let vk = PredicateKey::always_accept();
    let initial_state_root = Hash::from([1u8; 32]);
    let snark_state = OLSnarkAccountState::new_fresh(vk, initial_state_root);
    let balance = BitcoinAmount::from_sat(initial_balance);
    let new_acct_data = NewAccountData::new(balance, AccountTypeState::Snark(snark_state));
    let snark_serial = state
        .create_new_account(snark_id, new_acct_data)
        .expect("Should create snark account");

    let genesis_info = BlockInfo::new_genesis(1_000_000);
    let genesis_components = BlockComponents::new_empty();
    let genesis_block = execute_block(state, &genesis_info, None, genesis_components)
        .expect("Genesis should execute");

    (genesis_block.header().clone(), snark_serial)
}

/// Helper to create additional empty accounts (for testing transfers/messages)
fn create_empty_account(state: &mut OLState, account_id: AccountId) -> AccountSerial {
    let empty_state = AccountTypeState::Empty;
    let new_acct_data = NewAccountData::new_empty(empty_state);
    state
        .create_new_account(account_id, new_acct_data)
        .expect("Should create empty account")
}

/// Helper to create a basic snark account update transaction
/// Queries the current state to determine inbox index and processes all pending messages
fn create_update_tx(
    state: &OLState,
    snark_id: AccountId,
) -> (&NativeAccountState, &NativeSnarkAccountState) {
    let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
    (snark_account, snark_account.as_snark_account().unwrap())
}

// === Test Modules ===

/// Tests for basic validation errors like sequence numbers, balance checks, and recipient
/// validation
mod validation {
    use super::*;

    #[test]
    fn test_snark_update_invalid_sequence_number() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Try to submit update with wrong sequence number (should be 0, but we use 5)
        let invalid_tx = SnarkUpdateBuilder::new(5, get_test_state_root(2))
            .with_transfer(recipient_id, 10_000_000)
            .with_message_index(0) // Must set index for simple build
            .build_simple(snark_id);

        // Execute and expect failure
        let (slot, epoch) = (1, 0);
        let result =
            execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

        assert!(result.is_err(), "Update with wrong sequence should fail");
        match result.unwrap_err() {
            ExecError::Acct(AcctError::InvalidUpdateSequence { expected, got, .. }) => {
                assert_eq!(expected, 0);
                assert_eq!(got, 5);
            }
            err => panic!("Expected InvalidUpdateSequence, got: {err:?}"),
        }
    }

    #[test]
    fn test_snark_update_insufficient_balance() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account of only 50M sats
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 50_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Try to send 100M sats (more than balance)
        let invalid_tx = SnarkUpdateBuilder::new(0, get_test_state_root(2))
            .with_transfer(recipient_id, 100_000_000)
            .build(snark_id, &state);

        let (slot, epoch) = (1, 0);
        let result =
            execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

        assert!(
            result.is_err(),
            "Update with insufficient balance should fail"
        );
        match result.unwrap_err() {
            ExecError::Acct(AcctError::InsufficientBalance {
                requested,
                available,
            }) => {
                assert_eq!(requested, BitcoinAmount::from_sat(100_000_000));
                assert_eq!(available, BitcoinAmount::from_sat(50_000_000));
            }
            err => panic!("Expected InsufficientBalance, got: {err:?}"),
        }
    }

    #[test]
    fn test_snark_update_nonexistent_recipient() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let nonexistent_id = test_account_id(TEST_NONEXISTENT_ID); // Not created

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Try to send to non-existent account
        let invalid_tx = SnarkUpdateBuilder::new(0, get_test_state_root(2))
            .with_transfer(nonexistent_id, 10_000_000)
            .build(snark_id, &state);

        let (slot, epoch) = (1, 0);
        let result =
            execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

        assert!(
            result.is_err(),
            "Update to non-existent account should fail"
        );
        match result.unwrap_err() {
            ExecError::Acct(AcctError::MissingExpectedAccount(id)) => {
                assert_eq!(id, nonexistent_id);
            }
            err => panic!("Expected NonExistentAccount, got: {err:?}"),
        }
    }
}

/// Tests for inbox operations including message insertion, processing, and validation
mod inbox {
    use super::*;

    #[test]
    fn test_snark_inbox_message_insertion() {
        let mut state = OLState::new_genesis();
        let snark_id = test_account_id(100);

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Send a message to snark account via GAM(Generic Account Message) tx (from sequencer,
        // value=0)
        let msg_data = vec![1u8, 2, 3, 4, 5];

        // Create GAM transaction
        let gam_tx_payload =
            GamTxPayload::new(snark_id, msg_data.clone()).expect("Should create GAM payload");
        let gam_tx = TransactionPayload::GenericAccountMessage(gam_tx_payload);

        // Execute transaction
        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), gam_tx, slot, epoch);
        assert!(
            result.is_ok(),
            "GAM transaction should succeed: {:?}",
            result.err()
        );

        // Verify the message was added to inbox
        let (snark_account, snark_state) = get_snark_state_expect(&state, snark_id);

        // Check that inbox MMR now has 1 entry (from GAM)
        assert_eq!(
            snark_state.inbox_mmr().num_entries(),
            1,
            "Inbox should have 1 message (GAM)"
        );

        // Check the seq no of the sender
        assert_eq!(
            *snark_account.as_snark_account().unwrap().seqno().inner(),
            0,
            "Sender account seq no should not increase for GAM"
        );

        // Balance unchanged (GAM messages have 0 value)
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(100_000_000),
            "Snark account balance should be unchanged"
        );
    }

    #[test]
    fn test_snark_update_process_inbox_message_with_valid_mmr_proof() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Create parallel MMR tracker to generate proofs
        let mut inbox_tracker = InboxMmrTracker::new();

        // Step 1: Send a message to snark account inbox
        let msg_data = vec![1u8, 2, 3, 4];
        let gam_tx = TransactionPayload::GenericAccountMessage(
            GamTxPayload::new(snark_id, msg_data.clone()).expect("Should create GAM payload"),
        );
        let (slot, epoch) = (1, 0);
        let blk1 = execute_tx_in_block(&mut state, genesis_block.header(), gam_tx, slot, epoch)
            .expect("GAM should succeed");
        let header = blk1.header();

        // Track the message in parallel MMR (must match exactly what was inserted)
        let gam_msg_entry = MessageEntry::new(
            SEQUENCER_ACCT_ID,
            epoch, // epoch when message was added
            MsgPayload::new(BitcoinAmount::from_sat(0), msg_data),
        );

        let gam_proof = inbox_tracker.add_message(&gam_msg_entry);

        // Step 2: Verify the parallel MMR matches the actual inbox MMR
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        let snark_state = snark_account.as_snark_account().unwrap();

        assert_eq!(
            snark_state.inbox_mmr().num_entries(),
            inbox_tracker.num_entries(),
            "Parallel MMR must stay synchronized with actual inbox MMR"
        );
        assert_eq!(snark_state.inbox_mmr().num_entries(), 1);

        // The snark account starts with next_msg_read_idx = 0 (no messages processed yet)
        assert_eq!(snark_state.next_inbox_msg_idx(), 0);

        // Step 3: Create update that indicates that the GAM message was processed.
        let outputs = UpdateOutputs::new(
            vec![OutputTransfer::new(
                recipient_id,
                BitcoinAmount::from_sat(10_000_000),
            )],
            vec![],
        );

        // After processing 1 message starting at index 0, next_msg_read_idx should be 1
        let new_proof_state = ProofState::new(get_test_state_root(2), 1);
        let operation_data = UpdateOperationData::new(
            0, // seq_no
            new_proof_state,
            vec![gam_msg_entry], // processed_messages
            LedgerRefs::new_empty(),
            outputs,
            vec![],
        );

        let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(
            vec![gam_proof], // inbox_proofs
            LedgerRefProofs::new(vec![]),
        );
        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let update_tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
            snark_id,
            update_container,
        ));

        // Step 4: Execute the update
        let (slot, epoch) = (2, 0);
        let result = execute_tx_in_block(&mut state, header, update_tx, slot, epoch);
        assert!(
            result.is_ok(),
            "Update with valid message proof should succeed: {:?}",
            result.err()
        );

        // Verify the update was applied
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(90_000_000),
            "Sender account should be debited"
        );

        assert_eq!(
            *snark_account.as_snark_account().unwrap().seqno().inner(),
            1,
            "Sender seq no should increment"
        );

        let snark_state = snark_account.as_snark_account().unwrap();
        assert_eq!(
            snark_state.next_inbox_msg_idx(),
            1,
            "Next inbox msg index should increment"
        );

        let recipient_account = state.get_account_state(recipient_id).unwrap().unwrap();
        assert_eq!(
            recipient_account.balance(),
            BitcoinAmount::from_sat(10_000_000),
            "Recipient should receive transfer"
        );
    }

    #[test]
    fn test_snark_update_invalid_message_index() {
        let mut state = OLState::new_genesis();
        let snark_id = test_account_id(100);
        let recipient_id = test_account_id(200);

        // Setup: genesis with snark account with balance (no deposit message)
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        let outputs = UpdateOutputs::new(
            vec![OutputTransfer::new(
                recipient_id,
                BitcoinAmount::from_sat(10_000_000),
            )],
            vec![],
        );

        // Create proof state claiming to have processed 5 messages (but inbox is empty)
        let new_proof_state = ProofState::new(get_test_state_root(2), 5); // Claim we're at idx 5
        let operation_data = UpdateOperationData::new(
            0, // the first update, seq_no = 0
            new_proof_state,
            vec![], // No messages processed
            LedgerRefs::new_empty(),
            outputs,
            vec![],
        );

        let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let invalid_tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
            snark_id,
            update_container,
        ));

        let (slot, epoch) = (1, 0);
        let result =
            execute_tx_in_block(&mut state, genesis_block.header(), invalid_tx, slot, epoch);

        assert!(
            result.is_err(),
            "Update with wrong message index should fail"
        );
        match result.unwrap_err() {
            ExecError::Acct(AcctError::InvalidMsgIndex { expected, got, .. }) => {
                assert_eq!(expected, 0); // Should stay at 0
                assert_eq!(got, 5); // But claimed 5
            }
            err => panic!("Expected InvalidMsgIndex, got: {err:?}"),
        }
    }

    #[test]
    fn test_snark_update_invalid_message_proof() {
        let mut state = OLState::new_genesis();
        let snark_id = test_account_id(100);

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Step 1: Send a gam message to snark's inbox
        let msg_data = vec![1u8, 2, 3, 4];
        let gam_tx = TransactionPayload::GenericAccountMessage(
            GamTxPayload::new(snark_id, msg_data.clone()).expect("Should create GAM payload"),
        );
        let (slot, epoch) = (1, 0);
        let blk = execute_tx_in_block(&mut state, genesis_block.header(), gam_tx, slot, epoch)
            .expect("GAM should succeed");
        let header = blk.header();

        let (_, snark_state) = get_snark_state_expect(&state, snark_id);
        assert_eq!(
            snark_state.inbox_mmr().num_entries(),
            1,
            "1 inbox msg entry after gam message tx "
        );
        assert_eq!(
            snark_state.next_inbox_msg_idx(),
            0,
            "next to be processed msg idx should be 0"
        );

        // Step 2: Create update with INVALID proof for the gam message (index 0)
        // First create msg entry
        let deposit_msg = MessageEntry::new(
            BRIDGE_GATEWAY_ACCT_ID,
            0,
            MsgPayload::new(BitcoinAmount::from(0), msg_data),
        );

        // Create an invalid proof with bogus cohashes
        let invalid_raw_proof = RawMerkleProof {
            cohashes: vec![FixedBytes::<32>::from([0xff; 32])].into(),
        };
        let invalid_msg_proof = MessageEntryProof::new(deposit_msg.clone(), invalid_raw_proof);

        // Create update
        let outputs = UpdateOutputs::new(vec![], vec![]);

        let new_msg_idx = 1;
        let new_proof_state = ProofState::new(get_test_state_root(2), new_msg_idx);
        let operation_data = UpdateOperationData::new(
            0,
            new_proof_state,
            vec![deposit_msg],
            LedgerRefs::new_empty(),
            outputs,
            vec![],
        );

        let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
        let accumulator_proofs =
            UpdateAccumulatorProofs::new(vec![invalid_msg_proof], LedgerRefProofs::new(vec![]));
        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let invalid_tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
            snark_id,
            update_container,
        ));

        // Step 3: Execute and expect failure
        let (slot, epoch) = (2, 0);
        let result = execute_tx_in_block(&mut state, header, invalid_tx, slot, epoch);

        assert!(
            result.is_err(),
            "Update with invalid message proof should fail"
        );
        match result.unwrap_err() {
            ExecError::Acct(AcctError::InvalidMessageProof { msg_idx, .. }) => {
                assert_eq!(msg_idx, 0, "Should fail on message index 0");
            }
            err => panic!("Expected InvalidMessageProof, got: {err:?}"),
        }
    }

    #[test]
    fn test_snark_update_skip_message_out_of_order() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Step 1: Send TWO messages to inbox
        let msg1_data = vec![1u8, 2, 3, 4];
        let gam_tx1 = TransactionPayload::GenericAccountMessage(
            GamTxPayload::new(snark_id, msg1_data.clone()).expect("Should create GAM payload"),
        );
        let (slot, epoch) = (1, 0);
        let blk = execute_tx_in_block(
            &mut state,
            genesis_block.header(),
            gam_tx1.clone(),
            slot,
            epoch,
        )
        .expect("GAM 1 should succeed");
        let header = blk.header();

        let msg2_data = vec![5u8, 6, 7, 8];
        let gam_tx2 = TransactionPayload::GenericAccountMessage(
            GamTxPayload::new(snark_id, msg2_data.clone()).expect("Should create GAM payload"),
        );
        let blk = execute_tx_in_block(&mut state, header, gam_tx2, slot + 1, epoch)
            .expect("GAM 2 should succeed");
        let header = blk.header();

        // Verify we have 2 messages (2 GAMs, no deposit)
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        let snark_state = snark_account.as_snark_account().unwrap();
        assert_eq!(snark_state.inbox_mmr().num_entries(), 2);

        // Step 2: Try to process only the SECOND message (skipping first)
        // This should fail because messages must be processed in order starting from index 0
        let msg2_entry = MessageEntry::new(
            SEQUENCER_ACCT_ID,
            0,
            MsgPayload::new(BitcoinAmount::from_sat(0), msg2_data),
        );

        // The proof would be for index 1, but we're at index 0
        // This will fail the message index check, not the proof check
        let outputs = UpdateOutputs::new(
            vec![OutputTransfer::new(
                recipient_id,
                BitcoinAmount::from_sat(10_000_000),
            )],
            vec![],
        );

        // Claiming to process 1 message but jumping to index 2 (skipping first GAM)
        let new_proof_state = ProofState::new(get_test_state_root(2), 2); // Skip to index 2
        let operation_data = UpdateOperationData::new(
            0,
            new_proof_state,
            vec![msg2_entry], // Only 1 message processed
            LedgerRefs::new_empty(),
            outputs,
            vec![],
        );

        let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let invalid_tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
            snark_id,
            update_container,
        ));

        // Step 3: Execute and expect failure
        let (slot, epoch) = (3, 0);
        let result = execute_tx_in_block(&mut state, header, invalid_tx, slot, epoch);

        assert!(result.is_err(), "Update skipping messages should fail");
        match result.unwrap_err() {
            ExecError::Acct(AcctError::InvalidMsgIndex { expected, got, .. }) => {
                assert_eq!(
                    expected, 1,
                    "Should expect index 1 (current 0 + 1 message processed)"
                );
                assert_eq!(got, 2, "But got index 2 (skipped from 0 to 2)");
            }
            err => panic!("Expected InvalidMsgIndex, got: {err:?}"),
        }
    }
}

/// Tests for successful update operations
mod updates {
    use super::*;

    #[test]
    fn test_snark_update_success_with_transfer() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Create valid update with transfer
        let transfer_amount = 30_000_000u64;
        let tx = SnarkUpdateBuilder::new(0, get_test_state_root(2))
            .with_transfer(recipient_id, transfer_amount)
            .build(snark_id, &state);

        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
        assert!(
            result.is_ok(),
            "Valid update should succeed: {:?}",
            result.err()
        );

        // Verify balances
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(70_000_000),
            "Sender account balance should be 100M - 30M"
        );
        // Check the seq no of the sender
        assert_eq!(
            *snark_account.as_snark_account().unwrap().seqno().inner(),
            1,
            "Sender account seq no should increase"
        );

        let recipient_account = state.get_account_state(recipient_id).unwrap().unwrap();
        assert_eq!(
            recipient_account.balance(),
            BitcoinAmount::from_sat(30_000_000),
            "Recipient should receive 30M"
        );
    }
}

/// Deposit-withdraw tests for end-to-end workflows
mod deposit_withdrawal {
    use super::*;

    #[test]
    fn test_snark_account_deposit_and_withdrawal() {
        // Start with empty genesis state
        let mut state = OLState::new_genesis();

        // Create a snark account in the state
        let snark_account_id = get_test_snark_account_id();
        let initial_state_root = Hash::from([1u8; 32]);

        // Create a NativeSnarkAccountState with always-accept predicate key for testing
        let vk = PredicateKey::always_accept();
        let snark_state = NativeSnarkAccountState::new_fresh(vk, initial_state_root);

        let new_acct_data = NewAccountData::new_empty(AccountTypeState::Snark(snark_state));
        let snark_serial = state
            .create_new_account(snark_account_id, new_acct_data)
            .expect("Should create snark account");

        // Create a genesis block with a manifest containing a deposit to the snark account
        let deposit_amount = 150_000_000u64; // 1.5 BTC in satoshis (must be enough to cover withdrawal)
        let dest_subject = SubjectId::from([42u8; 32]);

        // Create a deposit intent log in the manifest
        let deposit_log_data =
            DepositIntentLogData::new(snark_serial, dest_subject, deposit_amount);
        let deposit_log_payload =
            strata_codec::encode_to_vec(&deposit_log_data).expect("Should encode deposit log data");

        // Create an ASM log entry with the deposit intent
        let deposit_log = AsmLogEntry::from_msg(
            strata_asm_manifest_types::DEPOSIT_INTENT_ASM_LOG_TYPE_ID,
            deposit_log_payload,
        )
        .expect("Should create deposit log");

        // Create manifest with the deposit log
        let genesis_manifest = AsmManifest::new(
            0,
            test_l1_block_id(1),
            WtxidsRoot::from(Buf32::from([0u8; 32])),
            vec![deposit_log],
        );

        // Execute genesis block with the deposit manifest
        let genesis_info = BlockInfo::new_genesis(1000000);
        let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);
        let genesis_output =
            execute_block_with_outputs(&mut state, &genesis_info, None, genesis_components)
                .expect("Genesis block should execute");
        let genesis_block = genesis_output.completed_block();

        // Verify the deposit was processed
        let account_after_deposit = state
            .get_account_state(snark_account_id)
            .expect("Should get account state")
            .expect("Account should exist");
        assert_eq!(
            account_after_deposit.balance(),
            BitcoinAmount::from_sat(deposit_amount),
            "Account balance should reflect the deposit"
        );

        // Check inbox state after genesis
        let snark_state_after_genesis = account_after_deposit.as_snark_account().unwrap();
        let nxt_inbox_idx_after_gen = snark_state_after_genesis.next_inbox_msg_idx();
        assert_eq!(
            nxt_inbox_idx_after_gen, 0,
            "Next inbox idx should still be zero"
        );
        debug!("Inbox MMR has {nxt_inbox_idx_after_gen} messages",);

        // Check the proof state (next message to PROCESS)
        let new_inner_st_root = snark_state_after_genesis.inner_state_root();
        debug!("New inner_state_root: {new_inner_st_root:?}");

        // Now create a snark account update transaction that produces a withdrawal
        let withdrawal_amount = 100_000_000u64; // Withdraw exactly 1 BTC (required denomination)
        let withdrawal_dest_desc = b"bc1qexample".to_vec(); // Example Bitcoin address descriptor
        let withdrawal_msg_data =
            WithdrawalMsgData::new(0, withdrawal_dest_desc.clone()).expect("Valid withdrawal data");

        // Encode the withdrawal message data using the msg-fmt library
        let encoded_withdrawal_body = strata_codec::encode_to_vec(&withdrawal_msg_data)
            .expect("Should encode withdrawal message");

        // Create OwnedMsg with proper format
        let withdrawal_msg = OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_withdrawal_body)
            .expect("Should create withdrawal message");

        // Convert to bytes for the MsgPayload
        let withdrawal_payload_data = withdrawal_msg.to_vec();

        // Create the withdrawal message payload (sent to bridge gateway)
        let withdrawal_payload = MsgPayload::new(
            BitcoinAmount::from_sat(withdrawal_amount),
            withdrawal_payload_data,
        );

        // Create the output message to the bridge gateway account
        let bridge_gateway_id = BRIDGE_GATEWAY_ACCT_ID;
        let output_message = OutputMessage::new(bridge_gateway_id, withdrawal_payload);

        // Create the update outputs with the withdrawal message
        let update_outputs = UpdateOutputs::new(vec![], vec![output_message]);

        // Create the snark account update operation data
        let seq_no = 0u64; // This is the first update.
        let new_state_root = get_test_state_root(2); // New state after update

        let account_after_genesis = state.get_account_state(snark_account_id).unwrap().unwrap();
        let snark_state_after_genesis = account_after_genesis.as_snark_account().unwrap();

        // Create a processed deposit message
        let deposit_msg = MessageEntry::new(
            BRIDGE_GATEWAY_ACCT_ID,
            1, // genesis epoch
            MsgPayload::new(BitcoinAmount::from_sat(deposit_amount), vec![]),
        );

        // After processing 1 message, next_msg_read_idx advances by 1
        let new_proof_state = ProofState::new(new_state_root, nxt_inbox_idx_after_gen + 1);

        let operation_data = UpdateOperationData::new(
            seq_no,
            new_proof_state.clone(),
            vec![deposit_msg],       // Processed deposit message
            LedgerRefs::new_empty(), // No ledger references
            update_outputs,
            vec![], // No extra data
        );

        // Create the snark account update
        let base_update = SnarkAccountUpdate::new(
            operation_data,
            vec![0u8; 32], // Dummy proof for testing
        );

        // Create accumulator proofs (empty for testing)
        let accumulator_proofs = UpdateAccumulatorProofs::new(
            vec![],                       // No inbox proofs
            LedgerRefProofs::new(vec![]), // No ledger ref proofs
        );

        // Create the update container
        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);

        // Create the snark account update transaction
        let sau_tx_payload = SnarkAccountUpdateTxPayload::new(snark_account_id, update_container);
        let sau_tx = TransactionPayload::SnarkAccountUpdate(sau_tx_payload);

        // Create block 1 with the withdrawal transaction
        let block1_info = BlockInfo::new(1001000, 1, 1);
        let block1_components = BlockComponents::new_txs(vec![sau_tx]);
        let block1_output = execute_block_with_outputs(
            &mut state,
            &block1_info,
            Some(genesis_block.header()),
            block1_components,
        )
        .expect("Block 1 should execute");

        let block1 = block1_output.completed_block();

        // Verify the withdrawal was processed
        let account_after_withdrawal = state
            .get_account_state(snark_account_id)
            .expect("Should get account state")
            .expect("Account should exist");

        // Balance should be reduced by withdrawal amount
        let expected_balance = deposit_amount - withdrawal_amount; // 150M - 100M = 50M satoshis
        assert_eq!(
            account_after_withdrawal.balance(),
            BitcoinAmount::from_sat(expected_balance),
            "Account balance should be reduced by withdrawal amount"
        );

        // Verify that logs were emitted
        let logs = block1_output.outputs().logs();
        let mut withdrawal_found = false;

        for log in logs {
            // Check if it's a withdrawal intent log (from the bridge gateway)
            if log.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL
                && let Ok(withdrawal_log) =
                    strata_codec::decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload())
            {
                withdrawal_found = true;

                // Verify the withdrawal details
                assert_eq!(
                    withdrawal_log.amt, withdrawal_amount,
                    "Withdrawal amount should match"
                );

                assert_eq!(
                    withdrawal_log.dest.as_slice(),
                    withdrawal_dest_desc.as_slice(),
                    "Withdrawal destination should match"
                );
            }
        }

        assert!(withdrawal_found, "test: missing withdrawal intent log");
    }
}

/// Tests for multiple operations in a single update
mod multi_operations {
    use super::*;

    #[test]
    fn test_snark_update_multiple_transfers() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient1_id = test_account_id(200);
        let recipient2_id = test_account_id(201);
        let recipient3_id = test_account_id(202);

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient accounts
        create_empty_account(&mut state, recipient1_id);
        create_empty_account(&mut state, recipient2_id);
        create_empty_account(&mut state, recipient3_id);

        // Create update with multiple transfers (30M + 20M + 10M = 60M total)
        let tx = SnarkUpdateBuilder::new(0, get_test_state_root(2))
            .with_transfer(recipient1_id, 30_000_000)
            .with_transfer(recipient2_id, 20_000_000)
            .with_transfer(recipient3_id, 10_000_000)
            .build(snark_id, &state);

        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
        assert!(
            result.is_ok(),
            "Multiple transfers should succeed: {:?}",
            result.err()
        );

        // Verify all balances
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(40_000_000),
            "Sender should have 100M - 60M = 40M"
        );

        let recipient1 = state.get_account_state(recipient1_id).unwrap().unwrap();
        assert_eq!(
            recipient1.balance(),
            BitcoinAmount::from_sat(30_000_000),
            "Recipient1 should receive 30M"
        );

        let recipient2 = state.get_account_state(recipient2_id).unwrap().unwrap();
        assert_eq!(
            recipient2.balance(),
            BitcoinAmount::from_sat(20_000_000),
            "Recipient2 should receive 20M"
        );

        let recipient3 = state.get_account_state(recipient3_id).unwrap().unwrap();
        assert_eq!(
            recipient3.balance(),
            BitcoinAmount::from_sat(10_000_000),
            "Recipient3 should receive 10M"
        );
    }

    #[test]
    fn test_snark_update_multiple_output_messages() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create multiple output messages
        let msg1_payload = MsgPayload::new(BitcoinAmount::from_sat(10_000_000), vec![1, 2, 3]);
        let msg2_payload = MsgPayload::new(BitcoinAmount::from_sat(5_000_000), vec![4, 5, 6]);
        let msg3_payload = MsgPayload::new(BitcoinAmount::from_sat(0), vec![7, 8, 9]);

        let output_message1 = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, msg1_payload);
        let output_message2 = OutputMessage::new(SEQUENCER_ACCT_ID, msg2_payload);
        let output_message3 = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, msg3_payload);

        // Create update with multiple messages
        let update_outputs = UpdateOutputs::new(
            vec![],
            vec![output_message1, output_message2, output_message3],
        );

        let seq_no = 0u64;
        let new_proof_state = ProofState::new(get_test_state_root(2), 0);
        let operation_data = UpdateOperationData::new(
            seq_no,
            new_proof_state,
            vec![],
            LedgerRefs::new_empty(),
            update_outputs,
            vec![],
        );

        let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
            snark_id,
            update_container,
        ));

        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
        assert!(
            result.is_ok(),
            "Multiple output messages should succeed: {:?}",
            result.err()
        );

        // Verify balance reduced by message values (10M + 5M + 0 = 15M)
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(85_000_000),
            "Balance should be reduced by total message value"
        );
    }

    #[test]
    fn test_snark_update_transfers_and_messages_combined() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Create update with both transfers and messages
        let transfer = OutputTransfer::new(recipient_id, BitcoinAmount::from_sat(25_000_000));
        let msg_payload = MsgPayload::new(BitcoinAmount::from_sat(15_000_000), vec![42, 43, 44]);
        let output_message = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, msg_payload);

        let update_outputs = UpdateOutputs::new(vec![transfer], vec![output_message]);

        let seq_no = 0u64;
        let new_proof_state = ProofState::new(get_test_state_root(2), 0);
        let operation_data = UpdateOperationData::new(
            seq_no,
            new_proof_state,
            vec![],
            LedgerRefs::new_empty(),
            update_outputs,
            vec![],
        );

        let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
            snark_id,
            update_container,
        ));

        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);
        assert!(
            result.is_ok(),
            "Combined transfers and messages should succeed: {:?}",
            result.err()
        );

        // Verify balances (100M - 25M - 15M = 60M)
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(60_000_000),
            "Sender should have 100M - 25M - 15M = 60M"
        );

        let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
        assert_eq!(
            recipient.balance(),
            BitcoinAmount::from_sat(25_000_000),
            "Recipient should receive 25M"
        );
    }

    #[test]
    fn test_snark_update_partial_balance_multiple_outputs() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient1_id = test_account_id(200);
        let recipient2_id = test_account_id(201);

        // Setup: genesis with snark account with 100M sats
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient accounts
        create_empty_account(&mut state, recipient1_id);
        create_empty_account(&mut state, recipient2_id);

        // Try to send 60M + 50M = 110M (exceeds balance of 100M)
        let tx = SnarkUpdateBuilder::new(0, get_test_state_root(2))
            .with_transfer(recipient1_id, 60_000_000)
            .with_transfer(recipient2_id, 50_000_000)
            .build(snark_id, &state);

        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

        assert!(result.is_err(), "Update exceeding balance should fail");
        match result.unwrap_err() {
            ExecError::Acct(AcctError::InsufficientBalance {
                requested,
                available,
            }) => {
                assert_eq!(requested, BitcoinAmount::from_sat(110_000_000));
                assert_eq!(available, BitcoinAmount::from_sat(100_000_000));
            }
            err => panic!("Expected InsufficientBalance, got: {err:?}"),
        }

        // Verify no partial execution - all balances should be unchanged
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(100_000_000),
            "Sender balance should be unchanged"
        );

        let recipient1 = state.get_account_state(recipient1_id).unwrap().unwrap();
        assert_eq!(
            recipient1.balance(),
            BitcoinAmount::from_sat(0),
            "Recipient1 should have no balance"
        );

        let recipient2 = state.get_account_state(recipient2_id).unwrap().unwrap();
        assert_eq!(
            recipient2.balance(),
            BitcoinAmount::from_sat(0),
            "Recipient2 should have no balance"
        );
    }
}

/// Tests for edge cases in value transfers
mod edge_cases {
    use super::*;

    #[test]
    fn test_snark_update_zero_value_transfer() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Create update with zero value transfer
        let tx = SnarkUpdateBuilder::new(0, get_test_state_root(2))
            .with_transfer(recipient_id, 0) // Zero value
            .build(snark_id, &state);

        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

        // Should succeed - zero transfers are valid
        assert!(
            result.is_ok(),
            "Zero value transfer should succeed: {:?}",
            result.err()
        );

        // Verify balances unchanged
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(100_000_000),
            "Sender balance should be unchanged"
        );
        assert_eq!(
            *snark_account.as_snark_account().unwrap().seqno().inner(),
            1,
            "Sequence number should still increment"
        );

        let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
        assert_eq!(
            recipient.balance(),
            BitcoinAmount::from_sat(0),
            "Recipient balance should remain 0"
        );
    }

    #[test]
    fn test_snark_update_self_transfer() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create update transferring to self
        let tx = SnarkUpdateBuilder::new(0, get_test_state_root(2))
            .with_transfer(snark_id, 30_000_000) // Transfer to self
            .build(snark_id, &state);

        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

        assert!(
            result.is_ok(),
            "Self transfer should succeed: {:?}",
            result.err()
        );

        // Verify balance unchanged (sent 30M to self)
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(100_000_000),
            "Balance should be unchanged after self-transfer"
        );
        assert_eq!(
            *snark_account.as_snark_account().unwrap().seqno().inner(),
            1,
            "Sequence number should increment"
        );
    }

    #[test]
    fn test_snark_update_exact_balance_transfer() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Transfer exactly the entire balance
        let tx = SnarkUpdateBuilder::new(0, get_test_state_root(2))
            .with_transfer(recipient_id, 100_000_000) // Entire balance
            .build(snark_id, &state);

        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

        assert!(
            result.is_ok(),
            "Exact balance transfer should succeed: {:?}",
            result.err()
        );

        // Verify balances
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(0),
            "Sender should have 0 balance"
        );

        let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
        assert_eq!(
            recipient.balance(),
            BitcoinAmount::from_sat(100_000_000),
            "Recipient should receive entire balance"
        );
    }

    #[test]
    fn test_snark_update_max_bitcoin_supply() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account with maximum Bitcoin supply
        // Bitcoin max supply is 21M BTC = 2.1 quadrillion satoshis
        let max_bitcoin_sats = 2_100_000_000_000_000u64; // 21M BTC in sats
        let genesis_block =
            setup_genesis_with_snark_account(&mut state, snark_id, max_bitcoin_sats);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Try multiple transfers that would exceed total Bitcoin supply
        let transfer1 =
            OutputTransfer::new(recipient_id, BitcoinAmount::from_sat(max_bitcoin_sats));
        let transfer2 = OutputTransfer::new(recipient_id, BitcoinAmount::from_sat(1)); // Even 1 sat more exceeds balance

        let update_outputs = UpdateOutputs::new(vec![transfer1, transfer2], vec![]);

        let seq_no = 0u64;
        let new_proof_state = ProofState::new(get_test_state_root(2), 0);
        let operation_data = UpdateOperationData::new(
            seq_no,
            new_proof_state,
            vec![],
            LedgerRefs::new_empty(),
            update_outputs,
            vec![],
        );

        let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);
        let accumulator_proofs = UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![]));
        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
            snark_id,
            update_container,
        ));

        let (slot, epoch) = (1, 0);
        let result = execute_tx_in_block(&mut state, genesis_block.header(), tx, slot, epoch);

        // Should fail due to insufficient balance
        assert!(result.is_err(), "Update exceeding balance should fail");

        match result.unwrap_err() {
            ExecError::Acct(AcctError::InsufficientBalance {
                requested,
                available,
            }) => {
                assert_eq!(requested, BitcoinAmount::from_sat(max_bitcoin_sats + 1));
                assert_eq!(available, BitcoinAmount::from_sat(max_bitcoin_sats));
            }
            err => panic!("Expected InsufficientBalance, got: {err:?}"),
        }

        // Verify no state change
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(max_bitcoin_sats),
            "Balance should be unchanged after failed update"
        );
    }
}

/// Tests for ledger references (referencing ASM manifests)
mod ledger_references {
    use strata_acct_types::tree_hash::TreeHash;

    use super::*;
    use crate::test_utils::ManifestMmrTracker;

    #[test]
    fn test_snark_update_with_valid_ledger_reference() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();
        let recipient_id = get_test_recipient_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create recipient account
        create_empty_account(&mut state, recipient_id);

        // Create parallel MMR tracker for manifests
        let mut manifest_tracker = ManifestMmrTracker::new();

        // Step 1: Execute a block with an ASM manifest to populate the state MMR
        let manifest1 = AsmManifest::new(
            1,
            test_l1_block_id(1),
            WtxidsRoot::from(Buf32::from([1u8; 32])),
            vec![], // No logs for simplicity
        );

        // Get the manifest hash before execution
        let manifest1_hash = <AsmManifest as TreeHash>::tree_hash_root(&manifest1);

        // Execute block with manifest
        let block1_info = BlockInfo::new(1001000, 1, 0); // slot 1, epoch 0
        let block1_components = BlockComponents::new_manifests(vec![manifest1.clone()]);
        let block1_output = execute_block_with_outputs(
            &mut state,
            &block1_info,
            Some(genesis_block.header()),
            block1_components,
        )
        .expect("Block 1 should execute");

        // Track the manifest in parallel MMR after execution (matching what state did)
        let (manifest1_index, manifest1_proof) = manifest_tracker.add_manifest(&manifest1);

        // Verify the manifest was added to state MMR
        assert_eq!(
            state.asm_manifests_mmr().num_entries(),
            manifest_tracker.num_entries(),
            "State MMR should match tracker MMR"
        );
        assert_eq!(manifest1_index, 0, "First manifest should be at index 0");

        // Step 2: Create a snark update that references the manifest
        let ledger_refs = LedgerRefs::new(vec![AccumulatorClaim::new(
            manifest1_index,
            manifest1_hash.into_inner(),
        )]);

        // Create update with ledger reference and a transfer
        let transfer = OutputTransfer::new(recipient_id, BitcoinAmount::from_sat(10_000_000));
        let update_outputs = UpdateOutputs::new(vec![transfer], vec![]);

        let seq_no = 0u64;
        let new_proof_state = ProofState::new(get_test_state_root(2), 0);
        let operation_data = UpdateOperationData::new(
            seq_no,
            new_proof_state,
            vec![],
            ledger_refs,
            update_outputs,
            vec![],
        );

        let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);

        // Include the valid proof for the ledger reference
        let accumulator_proofs =
            UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![manifest1_proof]));

        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
            snark_id,
            update_container,
        ));

        // Step 3: Execute the update
        let (slot, epoch) = (2, 1); // Increment epoch because we processed manifests in last
        // block
        let result = execute_tx_in_block(
            &mut state,
            block1_output.completed_block().header(),
            tx,
            slot,
            epoch,
        );

        assert!(
            result.is_ok(),
            "Update with valid ledger reference should succeed: {:?}",
            result.err()
        );

        // Verify the transfer was applied
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(90_000_000),
            "Sender balance should be reduced"
        );

        let recipient = state.get_account_state(recipient_id).unwrap().unwrap();
        assert_eq!(
            recipient.balance(),
            BitcoinAmount::from_sat(10_000_000),
            "Recipient should receive transfer"
        );
    }

    #[test]
    fn test_snark_update_with_invalid_ledger_reference() {
        let mut state = OLState::new_genesis();
        let snark_id = get_test_snark_account_id();

        // Setup: genesis with snark account
        let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

        // Create parallel MMR tracker
        let mut manifest_tracker = ManifestMmrTracker::new();

        // Step 1: Execute a block with an ASM manifest
        let manifest1 = AsmManifest::new(
            1,
            test_l1_block_id(1),
            WtxidsRoot::from(Buf32::from([1u8; 32])),
            vec![],
        );

        // Get the manifest hash before execution
        let manifest1_hash = <AsmManifest as TreeHash>::tree_hash_root(&manifest1);

        // Execute block with manifest
        let block1_info = BlockInfo::new(1001000, 1, 0); // slot 1, epoch 0
        let block1_components = BlockComponents::new_manifests(vec![manifest1.clone()]);
        let block1_output = execute_block_with_outputs(
            &mut state,
            &block1_info,
            Some(genesis_block.header()),
            block1_components,
        )
        .expect("Block 1 should execute");

        // Track the manifest in parallel MMR after execution (matching what state did)
        let (manifest1_index, _valid_proof) = manifest_tracker.add_manifest(&manifest1);

        // Step 2: Create a snark update with INVALID ledger reference proof
        let ledger_refs = LedgerRefs::new(vec![AccumulatorClaim::new(
            manifest1_index,
            manifest1_hash.into_inner(),
        )]);

        // Create an invalid proof with wrong cohashes
        let invalid_proof = MmrEntryProof::new(
            manifest1_hash.into_inner(),
            strata_acct_types::MerkleProof::from_cohashes(
                vec![[0xff; 32]], // Invalid cohash
                manifest1_index,
            ),
        );

        // Create update with ledger reference
        let update_outputs = UpdateOutputs::new_empty();

        let seq_no = 0u64;
        let new_proof_state = ProofState::new(get_test_state_root(2), 0);
        let operation_data = UpdateOperationData::new(
            seq_no,
            new_proof_state,
            vec![],
            ledger_refs,
            update_outputs,
            vec![],
        );

        let base_update = SnarkAccountUpdate::new(operation_data, vec![0u8; 32]);

        // Include the INVALID proof
        let accumulator_proofs =
            UpdateAccumulatorProofs::new(vec![], LedgerRefProofs::new(vec![invalid_proof]));

        let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);
        let tx = TransactionPayload::SnarkAccountUpdate(SnarkAccountUpdateTxPayload::new(
            snark_id,
            update_container,
        ));

        // Step 3: Execute and expect failure
        let (slot, epoch) = (2, 1); // Increment epoch because we processed manifests in the last
        // block
        let result = execute_tx_in_block(
            &mut state,
            block1_output.completed_block().header(),
            tx,
            slot,
            epoch,
        );

        assert!(
            result.is_err(),
            "Update with invalid ledger reference should fail"
        );

        match result.unwrap_err() {
            ExecError::Acct(AcctError::InvalidLedgerReference { ref_idx, .. }) => {
                assert_eq!(
                    ref_idx, manifest1_index,
                    "Should fail on the invalid reference"
                );
            }
            err => panic!("Expected InvalidLedgerReference, got: {err:?}"),
        }

        // Verify no state change
        let snark_account = state.get_account_state(snark_id).unwrap().unwrap();
        assert_eq!(
            snark_account.balance(),
            BitcoinAmount::from_sat(100_000_000),
            "Balance should be unchanged after failed update"
        );
    }
}
