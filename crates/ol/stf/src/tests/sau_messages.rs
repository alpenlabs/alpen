//! Tests for SAU output message behavior.

use strata_acct_types::{AccountId, BitcoinAmount};
use strata_ledger_types::{ISnarkAccountState, IStateAccessor};

use crate::{BRIDGE_GATEWAY_ACCT_ID, test_utils::*};

#[test]
fn test_snark_update_multiple_output_messages() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(TEST_RECIPIENT_ID + 1);
    let recipient2_id = make_account_id(TEST_RECIPIENT_ID + 2);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_snark_account(recipient1_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(0))
        })
        .with_genesis_snark_account(recipient2_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(0))
        })
        .execute_genesis();
    let limbo_before = fixture.state().limbo_funds();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.output_message(
                recipient1_id,
                BitcoinAmount::from_sat(10_000_000),
                vec![1, 2, 3],
            )
            .output_message(
                recipient2_id,
                BitcoinAmount::from_sat(5_000_000),
                vec![4, 5, 6],
            )
            // Bridge-gateway non-withdrawal messages are dropped after debiting the sender.
            .output_message(
                BRIDGE_GATEWAY_ACCT_ID,
                BitcoinAmount::from_sat(0),
                vec![7, 8, 9],
            )
            .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(85_000_000),
        "Balance should be reduced by total message value"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    assert_snark_received_message(&fixture, recipient1_id, 10_000_000);
    assert_snark_received_message(&fixture, recipient2_id, 5_000_000);

    let limbo_after = fixture.state().limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        0,
        "Zero-value malformed bridge-gateway message should not grow limbo"
    );
}

#[test]
fn test_snark_update_transfers_and_messages_combined() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);
    let message_recipient_id = make_account_id(TEST_RECIPIENT_ID + 3);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .with_genesis_snark_account(message_recipient_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(0))
        })
        .execute_genesis();
    let limbo_before = fixture.state().limbo_funds();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(25_000_000))
                .output_message(
                    message_recipient_id,
                    BitcoinAmount::from_sat(10_000_000),
                    vec![42, 43, 44],
                )
                // Bridge-gateway non-withdrawal messages are dropped after debiting the sender.
                .output_message(
                    BRIDGE_GATEWAY_ACCT_ID,
                    BitcoinAmount::from_sat(5_000_000),
                    vec![45, 46, 47],
                )
                .with_state_root(make_state_root(2))
        })
        .execute();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(60_000_000),
        "Sender should have 100M - 25M - 15M = 60M"
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1,
        "Sender account seq no should increase"
    );

    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(25_000_000),
        "Recipient should receive 25M"
    );

    assert_snark_received_message(&fixture, message_recipient_id, 10_000_000);

    let limbo_after = fixture.state().limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        5_000_000,
        "Malformed bridge-gateway message value should be swept into limbo"
    );
}

fn assert_snark_received_message(
    fixture: &OLStfFixture,
    recipient_id: AccountId,
    expected_balance_sat: u64,
) {
    let account_state = fixture.expect_snark_account(recipient_id);
    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(expected_balance_sat),
        "message recipient balance should increase"
    );
    assert_eq!(
        account_state.inbox_mmr().num_entries(),
        1,
        "message recipient inbox should receive one entry"
    );
    assert_eq!(
        account_state.next_inbox_msg_idx(),
        0,
        "received messages should remain unprocessed"
    );
}
