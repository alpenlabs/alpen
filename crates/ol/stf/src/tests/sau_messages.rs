//! Tests for SAU output message behavior.

use strata_acct_types::BitcoinAmount;
use strata_ledger_types::IStateAccessor;

use crate::{BRIDGE_GATEWAY_ACCT_ID, SEQUENCER_ACCT_ID, test_utils::*};

#[test]
fn test_snark_update_multiple_output_messages() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();
    let limbo_before = fixture.state().limbo_funds();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.output_message(
                BRIDGE_GATEWAY_ACCT_ID,
                BitcoinAmount::from_sat(10_000_000),
                vec![1, 2, 3],
            )
            .output_message(
                SEQUENCER_ACCT_ID,
                BitcoinAmount::from_sat(5_000_000),
                vec![4, 5, 6],
            )
            .output_message(
                BRIDGE_GATEWAY_ACCT_ID,
                BitcoinAmount::from_sat(0),
                vec![7, 8, 9],
            )
            .with_state_root(make_state_root(2))
        })
        .execute();

    // Verify balance reduced by message values (10M + 5M + 0 = 15M)
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(85_000_000),
        "Balance should be reduced by total message value"
    );

    // All three output messages target the bridge gateway account
    // (SEQUENCER_ACCT_ID aliases BRIDGE_GATEWAY_ACCT_ID) with payload data
    // that does not parse as a valid MsgRef, so all three sweep their
    // values into limbo (10M + 5M + 0).
    let limbo_after = fixture.state().limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        15_000_000,
        "Limbo should grow by total of malformed bridge-gateway message values"
    );
}

#[test]
fn test_snark_update_transfers_and_messages_combined() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();
    let limbo_before = fixture.state().limbo_funds();

    fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.transfer(recipient_id, BitcoinAmount::from_sat(25_000_000))
                .output_message(
                    BRIDGE_GATEWAY_ACCT_ID,
                    BitcoinAmount::from_sat(15_000_000),
                    vec![42, 43, 44],
                )
                .with_state_root(make_state_root(2))
        })
        .execute();

    // Verify balances (100M - 25M - 15M = 60M)
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(60_000_000),
        "Sender should have 100M - 25M - 15M = 60M"
    );

    assert_eq!(
        fixture.account_balance(recipient_id),
        BitcoinAmount::from_sat(25_000_000),
        "Recipient should receive 25M"
    );

    // The output message to the bridge gateway carries payload data
    // (`vec![42, 43, 44]`) that does not parse as a valid MsgRef, so its
    // 15M value is swept into limbo.
    let limbo_after = fixture.state().limbo_funds();
    assert_eq!(
        limbo_after.to_sat() - limbo_before.to_sat(),
        15_000_000,
        "Limbo should grow by malformed bridge-gateway message value"
    );
}
