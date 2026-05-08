//! Tests for edge cases in value transfers

use strata_acct_types::BitcoinAmount;
use strata_ledger_types::ISnarkAccountState;

use crate::{BRIDGE_GATEWAY_ACCT_ID, errors::ExecError, test_utils::*};

#[test]
fn test_snark_update_zero_value_transfer() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(0))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(100_000_000),
        "Sender balance should be unchanged"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_id).seqno().inner(),
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
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(0))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(1))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    assert_eq!(
        *fixture.expect_snark_account(snark_id).seqno().inner(),
        0,
        "Sequence number should not increment on failed transfer"
    );

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(0))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        *fixture.expect_snark_account(snark_id).seqno().inner(),
        1,
        "Sequence number should increment even for zero transfer"
    );
    assert_eq!(
        fixture.account_balance(snark_id),
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
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(0))
                .transfer(snark_id, BitcoinAmount::from_sat(0))
                .output_message(BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount::from_sat(0), vec![])
                .with_state_root(make_state_root(3))
        })
        .execute();

    assert_eq!(
        *fixture.expect_snark_account(snark_id).seqno().inner(),
        2,
        "Sequence number should increment to 2"
    );
}

#[test]
fn test_snark_update_self_transfer() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(snark_id, BitcoinAmount::from_sat(30_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(100_000_000),
        "Balance should be unchanged after self-transfer"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
}

#[test]
fn test_snark_update_exact_balance_transfer() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(100_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(0),
        "Sender should have 0 balance"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(100_000_000),
        "Recipient should receive entire balance"
    );
}

#[test]
fn test_snark_update_max_bitcoin_supply() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let max_bitcoin_sats = 2_100_000_000_000_000u64;
    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(max_bitcoin_sats))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(max_bitcoin_sats))
                .transfer(recipient_id, BitcoinAmount::from_sat(1))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    assert_eq!(
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(max_bitcoin_sats),
        "Balance should be unchanged after failed update"
    );
}

#[test]
fn test_snark_update_single_transfer_exceeding_max_bitcoin_suceeds() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let more_than_max_bitcoin = 2_100_000_000_000_001u64;
    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(u64::MAX))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(more_than_max_bitcoin))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(u64::MAX - more_than_max_bitcoin),
        "Sender balance should be reduced by transfer amount"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(more_than_max_bitcoin),
        "Recipient should receive the transfer amount exceeding 21M BTC"
    );
}

#[test]
fn test_snark_update_overflow_u64_boundary() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 2);
    let initial_balance = u64::MAX - 100;

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(initial_balance))
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_id, |sau| {
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
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(initial_balance),
        "Balance should be unchanged after failed update"
    );

    let err = fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::from_sat(u64::MAX))
                .transfer(recipient2_id, BitcoinAmount::from_sat(1))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    assert!(
        matches!(err.into_base(), ExecError::AmountOverflow),
        "Expected AmountOverflow"
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

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(u64::MAX))
        })
        .with_genesis_empty_account(recipient1_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::from_sat(u64::MAX))
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(0),
        "Sender should have 0 balance after transferring u64::MAX"
    );
    assert_eq!(
        fixture.account_balance(recipient1_id),
        BitcoinAmount::from_sat(u64::MAX),
        "Recipient should receive u64::MAX"
    );
}
