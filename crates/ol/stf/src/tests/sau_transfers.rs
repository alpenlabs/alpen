//! Tests for SAU transfer behavior.

use strata_acct_types::{AcctError, BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount};
use strata_ledger_types::ISnarkAccountState;

use crate::{errors::ExecError, test_utils::*};

#[test]
fn test_snark_update_success_with_transfer() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let mut verify_state = fixture.state().clone();
    let parent_header = fixture.parent_header().clone();
    let outcome = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(30_000_000))
                .with_state_root(make_state_root(2))
                .with_proof(make_proof(1))
        })
        .execute();

    assert_verification_succeeds(
        &mut verify_state,
        outcome.completed_block().header(),
        Some(parent_header),
        outcome.completed_block().body(),
    );

    // Verify balances
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(70_000_000),
        "Sender account balance should be 100M - 30M"
    );
    // Check the seq no of the sender
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(30_000_000),
        "Recipient should receive 30M"
    );
}

#[test]
fn test_snark_update_multiple_transfers() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let recipient3_id = make_account_id(TEST_RECIPIENT_ID + 2);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .with_genesis_empty_account(recipient3_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::from_sat(30_000_000))
                .transfer(recipient2_id, BitcoinAmount::from_sat(20_000_000))
                .transfer(recipient3_id, BitcoinAmount::from_sat(10_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute();

    // Verify all balances
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(40_000_000),
        "Sender should have 100M - 60M = 40M"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    assert_eq!(
        fixture.account_balance(recipient1_id),
        BitcoinAmount::from_sat(30_000_000),
        "Recipient1 should receive 30M"
    );

    assert_eq!(
        fixture.account_balance(recipient2_id),
        BitcoinAmount::from_sat(20_000_000),
        "Recipient2 should receive 20M"
    );

    assert_eq!(
        fixture.account_balance(recipient3_id),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient3 should receive 10M"
    );
}

#[test]
fn test_snark_update_same_block_distinct_senders() {
    let sender1_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let sender2_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID + 1);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 2);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(sender1_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_snark_account(sender2_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(70_000_000))
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(sender1_acct_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::from_sat(10_000_000))
                .with_state_root(make_state_root(2))
        })
        .with_sau(sender2_acct_id, |sau| {
            sau.transfer(recipient2_id, BitcoinAmount::from_sat(20_000_000))
                .with_state_root(make_state_root(3))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(sender1_acct_id),
        BitcoinAmount::from_sat(90_000_000),
        "Sender1 balance should reflect its same-block transfer"
    );
    assert_eq!(
        *fixture
            .expect_snark_account(sender1_acct_id)
            .seqno()
            .inner(),
        1,
        "Sender1 sequence number should increment"
    );

    assert_eq!(
        fixture.account_balance(sender2_acct_id),
        BitcoinAmount::from_sat(50_000_000),
        "Sender2 balance should reflect its same-block transfer"
    );
    assert_eq!(
        *fixture
            .expect_snark_account(sender2_acct_id)
            .seqno()
            .inner(),
        1,
        "Sender2 sequence number should increment"
    );

    assert_eq!(
        fixture.account_balance(recipient1_id),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient1 should receive sender1 transfer"
    );

    assert_eq!(
        fixture.account_balance(recipient2_id),
        BitcoinAmount::from_sat(20_000_000),
        "Recipient2 should receive sender2 transfer"
    );
}

#[test]
fn test_snark_update_same_block_sequential_seqnos() {
    let sender_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 2);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(sender_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(sender_acct_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::from_sat(10_000_000))
                .with_state_root(make_state_root(2))
        })
        .with_sau(sender_acct_id, |sau| {
            sau.transfer(recipient2_id, BitcoinAmount::from_sat(20_000_000))
                .with_state_root(make_state_root(3))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(sender_acct_id),
        BitcoinAmount::from_sat(70_000_000),
        "Sender balance should include both same-block transfers"
    );
    assert_eq!(
        *fixture.expect_snark_account(sender_acct_id).seqno().inner(),
        2,
        "Sender sequence number should increment for both same-block updates"
    );

    assert_eq!(
        fixture.account_balance(recipient1_id),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient1 should receive the first transfer"
    );

    assert_eq!(
        fixture.account_balance(recipient2_id),
        BitcoinAmount::from_sat(20_000_000),
        "Recipient2 should receive the second transfer"
    );
}

#[test]
fn test_snark_update_same_block_duplicate_seqno_fails() {
    let sender_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 2);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(sender_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(sender_acct_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::from_sat(10_000_000))
                .with_state_root(make_state_root(2))
        })
        .with_sau(sender_acct_id, |sau| {
            sau.transfer(recipient2_id, BitcoinAmount::from_sat(20_000_000))
                .force_seqno(0)
                .with_state_root(make_state_root(3))
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidUpdateSequence {
            account_id,
            expected,
            got,
        }) => {
            assert_eq!(account_id, sender_acct_id);
            assert_eq!(expected, 1);
            assert_eq!(got, 0);
        }
        err => panic!("Expected InvalidUpdateSequence, got: {err:?}"),
    }

    assert_eq!(
        fixture.account_balance(sender_acct_id),
        BitcoinAmount::from_sat(90_000_000),
        "First same-block transfer should remain applied"
    );
    assert_eq!(
        *fixture.expect_snark_account(sender_acct_id).seqno().inner(),
        1,
        "Sender sequence number should reflect only the first update"
    );

    assert_eq!(
        fixture.account_balance(recipient1_id),
        BitcoinAmount::from_sat(10_000_000),
        "Recipient1 should receive the first transfer"
    );

    assert_eq!(
        fixture.account_balance(recipient2_id),
        BitcoinAmount::from_sat(0),
        "Recipient2 should not receive the duplicate-seqno transfer"
    );
}

#[test]
fn test_snark_update_partial_balance_multiple_outputs() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 1);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::from_sat(60_000_000))
                .transfer(recipient2_id, BitcoinAmount::from_sat(50_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    // Verify no partial execution - all balances should be unchanged
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(100_000_000),
        "Sender balance should be unchanged"
    );

    assert_eq!(
        fixture.account_balance(recipient1_id),
        BitcoinAmount::from_sat(0),
        "Recipient1 should have no balance"
    );

    assert_eq!(
        fixture.account_balance(recipient2_id),
        BitcoinAmount::from_sat(0),
        "Recipient2 should have no balance"
    );
}

#[test]
fn test_snark_update_zero_value_transfer() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(0))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(100_000_000),
        "Sender balance should be unchanged"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should still increment"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(0),
        "Recipient balance should remain 0"
    );
}

#[test]
fn test_snark_update_from_zero_balance_account() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(0))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(1))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        0,
        "Sequence number should not increment on failed transfer"
    );

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(0))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should increment even for zero transfer"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(0),
        "Balance should remain zero"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(0),
        "Recipient should have zero balance"
    );

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(0))
                .transfer(snark_acct_id, BitcoinAmount::from_sat(0))
                .output_message(BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount::from_sat(0), vec![])
                .with_state_root(make_state_root(3))
        })
        .execute();

    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        2,
        "Sequence number should increment to 2"
    );
}

#[test]
fn test_snark_update_self_transfer() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(snark_acct_id, BitcoinAmount::from_sat(30_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(100_000_000),
        "Balance should be unchanged after self-transfer"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
}

#[test]
fn test_snark_update_exact_balance_transfer() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);
    let second_recipient_id = make_account_id(TEST_RECIPIENT_ID + 1);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .with_genesis_empty_account(second_recipient_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(100_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(0),
        "Sender should have 0 balance"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(100_000_000),
        "Recipient should receive entire balance"
    );

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(second_recipient_id, BitcoinAmount::from_sat(1))
                .with_state_root(make_state_root(3))
        })
        .execute_err();

    assert!(
        matches!(err.into_base(), ExecError::BalanceUnderflow),
        "Expected BalanceUnderflow"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(0),
        "Sender balance should remain zero after failed transfer from drained balance"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should not increment after failed transfer from drained balance"
    );
    assert_eq!(
        fixture.account_balance(second_recipient_id),
        BitcoinAmount::from_sat(0),
        "Second recipient should not receive failed transfer from drained balance"
    );
}
