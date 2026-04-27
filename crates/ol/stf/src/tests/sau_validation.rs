//! Tests for snark account update validation errors.

use strata_acct_types::{AcctError, BitcoinAmount};
use strata_ledger_types::ISnarkAccountState;

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
fn test_snark_update_replay_across_blocks_fails() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let transfer_amount = 10_000_000;
    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let block1 = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(transfer_amount))
                .with_state_root(make_state_root(2))
        })
        .execute();
    let tx = block1
        .completed_block()
        .body()
        .tx_segment()
        .expect("first block should contain a tx segment")
        .txs()[0]
        .clone();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(90_000_000),
        "sender balance should reflect first execution"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "sender seqno should increment after first execution"
    );

    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(transfer_amount),
        "recipient should receive the first execution"
    );

    let err = fixture.child_block().with_tx(tx).execute_err();
    match err.into_base() {
        ExecError::Acct(AcctError::InvalidUpdateSequence {
            account_id,
            expected,
            got,
        }) => {
            assert_eq!(account_id, snark_acct_id);
            assert_eq!(expected, 1);
            assert_eq!(got, 0);
        }
        err => panic!("Expected InvalidUpdateSequence, got: {err:?}"),
    }

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(90_000_000),
        "sender balance should not change after replay failure"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "sender seqno should not change after replay failure"
    );

    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(transfer_amount),
        "recipient balance should not change after replay failure"
    );
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
