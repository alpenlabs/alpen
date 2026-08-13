//! Tests for SAU value and integer boundary behavior.

use strata_acct_types::{BitcoinAmount, MAX_MESSAGES, MAX_TRANSFERS};
use strata_ledger_types::ISnarkAccountState;
use strata_ol_chain_types::SAU_MAX_EXTRA_DATA_BYTES;

use crate::{errors::ExecError, test_utils::*};

const MAX_BITCOIN_MONEY_SATS: u64 = 21_000_000 * 100_000_000;

fn max_money() -> BitcoinAmount {
    BitcoinAmount::try_from(MAX_BITCOIN_MONEY_SATS)
        .expect("maximum Bitcoin money supply must be valid")
}

#[test]
fn test_snark_update_max_bitcoin_supply() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);
    let max_minus_one = BitcoinAmount::try_from(MAX_BITCOIN_MONEY_SATS - 1)
        .expect("amount below the maximum money supply must be valid");

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| acct.with_balance(max_minus_one))
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, max_money())
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    match err.into_base() {
        ExecError::BalanceUnderflow => {}
        err => panic!("Expected BalanceUnderflow, got: {err:?}"),
    }

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        max_minus_one,
        "Balance should be unchanged after failed update"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        0,
        "Sequence number should not increment after failed update"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        "Recipient should not receive failed update"
    );
}

#[test]
fn test_bitcoin_amount_rejects_value_above_max_supply() {
    assert!(BitcoinAmount::try_from(MAX_BITCOIN_MONEY_SATS + 1).is_err());
}

#[test]
fn test_bitcoin_amount_rejects_u64_max() {
    assert!(BitcoinAmount::try_from(u64::MAX).is_err());
}

#[test]
fn test_snark_update_rejects_aggregate_transfer_overflow() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 2);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| acct.with_balance(max_money()))
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient1_id, max_money())
                .transfer(
                    recipient2_id,
                    BitcoinAmount::try_from(1)
                        .expect("amount must not exceed the Bitcoin money supply"),
                )
                .with_state_root(make_state_root(2))
        })
        .execute_err();

    assert!(
        matches!(err.into_base(), ExecError::AmountOverflow),
        "Expected AmountOverflow"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        max_money(),
        "Balance should be unchanged after failed update"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        0,
        "Sequence number should not increment after failed update"
    );
    assert_eq!(
        fixture.account_balance(recipient1_id),
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        "Recipient1 should have no balance after failed update"
    );
    assert_eq!(
        fixture.account_balance(recipient2_id),
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        "Recipient2 should have no balance after failed update"
    );
}

#[test]
fn test_snark_update_allows_max_transfer_count() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID + 1);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(
                BitcoinAmount::try_from(MAX_TRANSFERS)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            let mut sau = sau;
            for _ in 0..MAX_TRANSFERS {
                sau = sau.transfer(
                    recipient_id,
                    BitcoinAmount::try_from(1)
                        .expect("amount must not exceed the Bitcoin money supply"),
                );
            }
            sau.with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        "Sender should spend the max-count transfer total"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::try_from(MAX_TRANSFERS)
            .expect("amount must not exceed the Bitcoin money supply"),
        "Recipient should receive every max-count transfer"
    );
}

#[test]
#[should_panic(expected = "test: too many transfer effects")]
fn test_snark_update_builder_rejects_transfer_count_over_limit() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID + 1);

    let fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
        })
        .execute_genesis();

    let mut builder =
        SnarkUpdateBuilder::from_snark_state(fixture.expect_snark_account(snark_acct_id).clone());
    for _ in 0..=MAX_TRANSFERS {
        builder = builder.with_transfer(recipient_id, 1);
    }
}

#[test]
fn test_snark_update_allows_max_message_count() {
    let sender_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID + 1);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(sender_acct_id, |acct| {
            acct.with_balance(
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
        })
        .with_genesis_snark_account(recipient_id, |acct| {
            acct.with_balance(
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
        })
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(sender_acct_id, |sau| {
            let mut sau = sau;
            for _ in 0..MAX_MESSAGES {
                sau = sau.output_message(
                    recipient_id,
                    BitcoinAmount::try_from(0)
                        .expect("amount must not exceed the Bitcoin money supply"),
                    vec![1, 2, 3],
                );
            }
            sau.with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        *fixture.expect_snark_account(sender_acct_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
    assert_eq!(
        fixture
            .expect_snark_account(recipient_id)
            .inbox_mmr()
            .num_entries(),
        MAX_MESSAGES,
        "Recipient inbox should receive every max-count message"
    );
}

#[test]
#[should_panic(expected = "test: too many message effects")]
fn test_snark_update_builder_rejects_message_count_over_limit() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID + 1);

    let fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
        })
        .execute_genesis();

    let mut builder =
        SnarkUpdateBuilder::from_snark_state(fixture.expect_snark_account(snark_acct_id).clone());
    for _ in 0..=MAX_MESSAGES {
        builder = builder.with_output_message(recipient_id, 0, vec![1, 2, 3]);
    }
}

#[test]
fn test_snark_update_allows_max_extra_data() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
        })
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_extra_data(vec![0xab; SAU_MAX_EXTRA_DATA_BYTES as usize])
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
    assert_eq!(
        fixture
            .expect_snark_account(snark_acct_id)
            .inner_state_root(),
        make_state_root(2),
        "Inner state root should update"
    );
}

#[test]
fn test_snark_update_builder_rejects_extra_data_over_limit() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(
                BitcoinAmount::try_from(0)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
        })
        .execute_genesis();

    let result =
        SnarkUpdateBuilder::from_snark_state(fixture.expect_snark_account(snark_acct_id).clone())
            .try_with_extra_data(vec![0xab; SAU_MAX_EXTRA_DATA_BYTES as usize + 1]);
    assert!(result.is_err(), "extra data over limit should fail");
}

#[test]
fn test_snark_update_allows_max_balance_transfer() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let second_recipient_id = make_account_id(TEST_RECIPIENT_ID + 2);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| acct.with_balance(max_money()))
        .with_genesis_empty_account(recipient_id)
        .with_genesis_empty_account(second_recipient_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, max_money())
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        "Sender should have 0 balance after transferring MAX_MONEY"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should increment"
    );
    assert_eq!(
        fixture.account_balance(recipient_id),
        max_money(),
        "Recipient should receive MAX_MONEY"
    );

    let err = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(
                second_recipient_id,
                BitcoinAmount::try_from(1)
                    .expect("amount must not exceed the Bitcoin money supply"),
            )
            .with_state_root(make_state_root(3))
        })
        .execute_err();

    assert!(
        matches!(err.into_base(), ExecError::BalanceUnderflow),
        "Expected BalanceUnderflow"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        "Sender balance should remain zero after failed transfer from drained balance"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sequence number should not increment after failed transfer from drained balance"
    );
    assert_eq!(
        fixture.account_balance(second_recipient_id),
        BitcoinAmount::try_from(0).expect("amount must not exceed the Bitcoin money supply"),
        "Second recipient should not receive failed transfer from drained balance"
    );
}
