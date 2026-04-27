//! Tests for SAU value and integer boundary behavior.

use strata_acct_types::BitcoinAmount;
use strata_ledger_types::ISnarkAccountState;

use crate::{errors::ExecError, test_utils::*};

#[test]
fn test_snark_update_max_bitcoin_supply() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::MAX_MONEY)
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::MAX_MONEY)
                .transfer(recipient_id, BitcoinAmount::from_sat(1))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::MAX_MONEY,
        "Balance should be unchanged after failed update"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        0,
        "Sequence number should not increment after failed update"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(0),
        "Recipient should not receive failed update"
    );
}

#[test]
fn test_snark_update_single_transfer_exceeding_max_bitcoin_suceeds() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let more_than_max_bitcoin = BitcoinAmount::from_sat(BitcoinAmount::MAX_MONEY.to_sat() + 1);
    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            // Intentionally above MAX_MONEY to pin current BitcoinAmount behavior.
            acct.with_balance(BitcoinAmount::MAX)
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, more_than_max_bitcoin)
                .with_state_root(make_state_root(2))
        })
        .execute();

    let expected_sender_balance = BitcoinAmount::MAX
        .checked_sub(more_than_max_bitcoin)
        .expect("MAX - (MAX_MONEY + 1) should not underflow");
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        expected_sender_balance,
        "Sender balance should be reduced by transfer amount"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        more_than_max_bitcoin,
        "Recipient should receive the transfer amount exceeding 21M BTC"
    );
}

#[test]
fn test_snark_update_overflow_u64_boundary() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 2);
    let initial_balance = u64::MAX - 100;

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(initial_balance))
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::from_sat(u64::MAX - 100))
                .transfer(recipient2_id, BitcoinAmount::from_sat(101))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    assert!(
        matches!(err.into_base(), ExecError::AmountOverflow),
        "Expected AmountOverflow"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(initial_balance),
        "Balance should be unchanged after failed update"
    );
    assert_eq!(
        fixture.account_balance(recipient1_id),
        BitcoinAmount::from_sat(0),
        "Recipient1 should have no balance after failed update"
    );
    assert_eq!(
        fixture.account_balance(recipient2_id),
        BitcoinAmount::from_sat(0),
        "Recipient2 should have no balance after failed update"
    );
}

#[test]
fn test_snark_update_rejects_aggregate_transfer_overflow() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 2);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            // Intentionally above MAX_MONEY to exercise SAU effect-math overflow.
            acct.with_balance(BitcoinAmount::MAX)
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::MAX)
                .transfer(recipient2_id, BitcoinAmount::from_sat(1))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    assert!(
        matches!(err.into_base(), ExecError::AmountOverflow),
        "Expected AmountOverflow"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::MAX,
        "Balance should be unchanged after failed update"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        0,
        "Sequence number should not increment after failed update"
    );
    assert_eq!(
        fixture.account_balance(recipient1_id),
        BitcoinAmount::from_sat(0),
        "Recipient1 should have no balance after failed update"
    );
    assert_eq!(
        fixture.account_balance(recipient2_id),
        BitcoinAmount::from_sat(0),
        "Recipient2 should have no balance after failed update"
    );
}

#[test]
fn test_snark_update_allows_max_balance_transfer() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let second_recipient_id = make_account_id(TEST_RECIPIENT_ID + 2);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::MAX_MONEY)
        })
        .with_genesis_empty_account(recipient_id)
        .with_genesis_empty_account(second_recipient_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::MAX_MONEY)
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(0),
        "Sender should have 0 balance after transferring MAX_MONEY"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::MAX_MONEY,
        "Recipient should receive MAX_MONEY"
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
