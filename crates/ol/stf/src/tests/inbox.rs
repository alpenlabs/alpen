//! Tests for inbox operations including message insertion, processing, and validation

use ssz_primitives::FixedBytes;
use strata_acct_types::{AcctError, BitcoinAmount, MessageEntry, MsgPayload, RawMerkleProof};
use strata_ledger_types::ISnarkAccountState;

use crate::{BRIDGE_GATEWAY_ACCT_ID, SEQUENCER_ACCT_ID, errors::ExecError, test_utils::*};

fn msg_payload_from_bytes(data: Vec<u8>) -> MsgPayload {
    MsgPayload::from_bytes(BitcoinAmount::from_sat(0), data)
        .expect("message payload bytes must fit within SSZ max length")
}

#[test]
fn test_snark_inbox_message_insertion() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    fixture
        .child_block()
        .with_default_gam(snark_acct_id)
        .execute();

    // Verify the message was added to inbox
    let account_state = fixture.expect_snark_account(snark_acct_id);

    // Check that inbox MMR now has 1 entry (from GAM)
    assert_eq!(
        account_state.inbox_mmr().num_entries(),
        1,
        "Inbox should have 1 message (GAM)"
    );

    // Check the seq no of the sender
    assert_eq!(
        *account_state.seqno().inner(),
        0,
        "Sender account seq no should not increase for GAM"
    );

    // Balance unchanged (GAM messages have 0 value)
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(100_000_000),
        "Snark account balance should be unchanged"
    );
}

#[test]
fn test_snark_update_process_inbox_message_with_valid_mmr_proof() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    // Create parallel MMR tracker to generate proofs
    let mut inbox_tracker = InboxMmrTracker::new();

    // Step 1: Send a message to snark account inbox
    let gam_output = fixture
        .child_block()
        .with_default_gam(snark_acct_id)
        .execute();

    // Track the message in parallel MMR (must match exactly what the STF inserted:
    // GAM produces an empty MsgPayload with 0 value and no data)
    let gam_msg_entry = MessageEntry::new(
        SEQUENCER_ACCT_ID,
        gam_output.completed_block().header().epoch(),
        MsgPayload::new_empty(),
    );

    let gam_proof = inbox_tracker.add_message(&gam_msg_entry);

    // Step 2: Verify the parallel MMR matches the actual inbox MMR
    let account_state = fixture.expect_snark_account(snark_acct_id);
    let prev_seq_no = *account_state.seqno().inner();

    assert_eq!(
        account_state.inbox_mmr().num_entries(),
        inbox_tracker.num_entries(),
        "Parallel MMR must stay synchronized with actual inbox MMR"
    );
    assert_eq!(account_state.inbox_mmr().num_entries(), 1);

    // The snark account starts with next_msg_read_idx = 0 (no messages processed yet)
    assert_eq!(account_state.next_inbox_msg_idx(), 0);

    let mut verify_state = fixture.state().clone();
    let parent_header = fixture.parent_header().clone();

    // Step 3: Create update that indicates that the GAM message was processed.
    let update_outcome = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_processed_messages(vec![gam_msg_entry], vec![gam_proof])
                .transfer(recipient_id, BitcoinAmount::from_sat(10_000_000))
                .with_state_root(make_state_root(2))
                .with_proof(vec![0u8; 32])
        })
        .execute();

    assert_verification_succeeds(
        &mut verify_state,
        update_outcome.completed_block().header(),
        Some(parent_header),
        update_outcome.completed_block().body(),
    );

    // Verify the update was applied
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(90_000_000),
        "Sender account should be debited"
    );

    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        prev_seq_no + 1,
        "Sender seq no should increment"
    );

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        account_state.next_inbox_msg_idx(),
        1,
        "Next inbox msg index should increment"
    );

    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient should receive transfer"
    );
}

#[test]
fn test_snark_update_invalid_message_index() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let snapshot = fixture.snapshot([snark_acct_id, recipient_id]);

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(10_000_000))
                .force_next_inbox_msg_idx(5)
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidMsgIndex { expected, got, .. }) => {
            assert_eq!(expected, 0); // Should stay at 0
            assert_eq!(got, 5); // But claimed 5
        }
        err => panic!("Expected InvalidMsgIndex, got: {err:?}"),
    }

    snapshot.assert_unchanged(&fixture);
}

#[test]
fn test_snark_update_invalid_message_proof() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    // Step 1: Send a gam message to snark's inbox
    fixture
        .child_block()
        .with_default_gam(snark_acct_id)
        .execute();

    // Verify the message was added to inbox
    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        account_state.inbox_mmr().num_entries(),
        1,
        "1 inbox msg entry after gam message tx "
    );
    assert_eq!(
        account_state.next_inbox_msg_idx(),
        0,
        "next to be processed msg idx should be 0"
    );
    let snapshot = fixture.snapshot([snark_acct_id]);

    // Step 2: Create update with INVALID proof for the gam message (index 0)
    // First create msg entry (deliberately using wrong source to keep it invalid)
    let deposit_msg = MessageEntry::new(BRIDGE_GATEWAY_ACCT_ID, 0, MsgPayload::new_empty());

    // Create an invalid proof with bogus cohashes
    let invalid_raw_proof = RawMerkleProof {
        cohashes: vec![FixedBytes::<32>::from([0xff; 32])]
            .try_into()
            .expect("single cohash should fit in raw proof"),
    };

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_processed_messages(vec![deposit_msg], vec![invalid_raw_proof])
                .with_state_root(make_state_root(2))
                .with_proof(vec![0u8; 32])
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidMessageProof { msg_idx, .. }) => {
            assert_eq!(msg_idx, 0, "Should fail on message index 0");
        }
        err => panic!("Expected InvalidMessageProof, got: {err:?}"),
    }

    snapshot.assert_unchanged(&fixture);
}

#[test]
fn test_snark_update_skip_message_out_of_order() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    // Step 1: Send TWO messages to inbox
    fixture
        .child_block()
        .with_default_gam(snark_acct_id)
        .execute();

    fixture
        .child_block()
        .with_default_gam(snark_acct_id)
        .execute();

    // Verify we have 2 messages (2 GAMs, no deposit)
    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(account_state.inbox_mmr().num_entries(), 2);
    let snapshot = fixture.snapshot([snark_acct_id, recipient_id]);

    // Step 2: Try to process only the SECOND message (skipping first)
    // This should fail because messages must be processed in order starting from index 0
    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(10_000_000))
                .force_next_inbox_msg_idx(2)
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidMsgIndex { expected, got, .. }) => {
            assert_eq!(
                expected, 0,
                "Should expect index 0 (current 0 + 0 messages processed)"
            );
            assert_eq!(got, 2, "But got index 2 (skipped from 0 to 2)");
        }
        err => panic!("Expected InvalidMsgIndex, got: {err:?}"),
    }

    snapshot.assert_unchanged(&fixture);
}

#[test]
fn test_snark_update_rejects_reversed_processed_messages() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);
    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let mut inbox_tracker = InboxMmrTracker::new();
    let first_output = fixture
        .child_block()
        .with_gam(snark_acct_id, |gam| gam.with_payload(vec![1]))
        .execute();
    let first_msg = MessageEntry::new(
        SEQUENCER_ACCT_ID,
        first_output.completed_block().header().epoch(),
        msg_payload_from_bytes(vec![1]),
    );
    let first_proof = inbox_tracker.add_message(&first_msg);

    let second_output = fixture
        .child_block()
        .with_gam(snark_acct_id, |gam| gam.with_payload(vec![2]))
        .execute();
    let second_msg = MessageEntry::new(
        SEQUENCER_ACCT_ID,
        second_output.completed_block().header().epoch(),
        msg_payload_from_bytes(vec![2]),
    );
    let second_proof = inbox_tracker.add_message(&second_msg);

    assert_eq!(
        fixture
            .expect_snark_account(snark_acct_id)
            .inbox_mmr()
            .num_entries(),
        2
    );

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_processed_messages(
                vec![second_msg, first_msg],
                vec![second_proof, first_proof],
            )
            .transfer(recipient_id, BitcoinAmount::from_sat(10_000_000))
            .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidMessageProof { msg_idx, .. }) => {
            assert_eq!(
                msg_idx, 0,
                "reversed order should fail at first inbox index"
            );
        }
        err => panic!("Expected InvalidMessageProof, got: {err:?}"),
    }

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(100_000_000)
    );
    assert_eq!(*account_state.seqno().inner(), 0);
    assert_eq!(account_state.next_inbox_msg_idx(), 0);
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(0)
    );
}

#[test]
fn test_snark_update_rejects_duplicate_processed_message() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);
    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let mut inbox_tracker = InboxMmrTracker::new();
    let first_output = fixture
        .child_block()
        .with_gam(snark_acct_id, |gam| gam.with_payload(vec![1]))
        .execute();
    let first_msg = MessageEntry::new(
        SEQUENCER_ACCT_ID,
        first_output.completed_block().header().epoch(),
        msg_payload_from_bytes(vec![1]),
    );
    inbox_tracker.add_message(&first_msg);

    let second_output = fixture
        .child_block()
        .with_gam(snark_acct_id, |gam| gam.with_payload(vec![2]))
        .execute();
    let second_msg = MessageEntry::new(
        SEQUENCER_ACCT_ID,
        second_output.completed_block().header().epoch(),
        msg_payload_from_bytes(vec![2]),
    );
    inbox_tracker.add_message(&second_msg);

    assert_eq!(
        fixture
            .expect_snark_account(snark_acct_id)
            .inbox_mmr()
            .num_entries(),
        2
    );
    let first_proof = inbox_tracker.expect_raw_proof_at(0);

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_processed_messages(
                vec![first_msg.clone(), first_msg],
                vec![first_proof.clone(), first_proof],
            )
            .transfer(recipient_id, BitcoinAmount::from_sat(10_000_000))
            .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidMessageProof { msg_idx, .. }) => {
            assert_eq!(
                msg_idx, 1,
                "duplicate message should fail at second inbox index"
            );
        }
        err => panic!("Expected InvalidMessageProof, got: {err:?}"),
    }

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(100_000_000)
    );
    assert_eq!(*account_state.seqno().inner(), 0);
    assert_eq!(account_state.next_inbox_msg_idx(), 0);
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(0)
    );
}
