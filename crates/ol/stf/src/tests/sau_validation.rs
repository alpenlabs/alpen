//! Tests for snark account update validation errors.

use strata_acct_types::{AcctError, BitcoinAmount};

use crate::{errors::ExecError, test_utils::*};

#[test]
fn test_snark_update_invalid_sequence_number() {
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
            sau.force_seqno(5)
                .transfer(recipient_id, BitcoinAmount::from_sat(10_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::Acct(AcctError::InvalidUpdateSequence { expected, got, .. }) => {
            assert_eq!(expected, 0);
            assert_eq!(got, 5);
        }
        err => panic!("Expected InvalidUpdateSequence, got: {err:?}"),
    }

    snapshot.assert_unchanged(&fixture);
}

#[test]
fn test_snark_update_insufficient_balance() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(50_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let snapshot = fixture.snapshot([snark_acct_id, recipient_id]);

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(100_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    snapshot.assert_unchanged(&fixture);
}

#[test]
fn test_snark_update_nonexistent_recipient() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let nonexistent_id = make_account_id(TEST_NONEXISTENT_ID); // Not created

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let snapshot = fixture.snapshot([snark_acct_id, nonexistent_id]);

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(nonexistent_id, BitcoinAmount::from_sat(10_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::UnknownAccount(id) => {
            assert_eq!(id, nonexistent_id);
        }
        err => panic!("Expected UnknownAccount, got: {err:?}"),
    }

    snapshot.assert_unchanged(&fixture);
}
