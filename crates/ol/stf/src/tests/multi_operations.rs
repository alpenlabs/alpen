//! Tests for multiple operations in a single update

use strata_acct_types::BitcoinAmount;
use strata_ledger_types::IStateAccessor;

use crate::{BRIDGE_GATEWAY_ACCT_ID, SEQUENCER_ACCT_ID, errors::ExecError, test_utils::*};

#[test]
fn test_snark_update_multiple_transfers() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(200);
    let recipient2_id = make_account_id(201);
    let recipient3_id = make_account_id(202);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .with_genesis_empty_account(recipient3_id)
        .execute_genesis();

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
            sau.transfer(recipient1_id, BitcoinAmount::from_sat(30_000_000))
                .transfer(recipient2_id, BitcoinAmount::from_sat(20_000_000))
                .transfer(recipient3_id, BitcoinAmount::from_sat(10_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute();

    // Verify all balances
    assert_eq!(
        fixture.account_balance(snark_id),
        BitcoinAmount::from_sat(40_000_000),
        "Sender should have 100M - 60M = 40M"
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
fn test_snark_update_multiple_output_messages() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();
    let limbo_before = fixture.state().limbo_funds();

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
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
        fixture.account_balance(snark_id),
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
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient_id = make_account_id(TEST_RECIPIENT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient_id)
        .execute_genesis();
    let limbo_before = fixture.state().limbo_funds();

    fixture
        .child_block()
        .with_sau(snark_id, |sau| {
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
        fixture.account_balance(snark_id),
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

#[test]
fn test_snark_update_partial_balance_multiple_outputs() {
    let snark_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let recipient1_id = make_account_id(200);
    let recipient2_id = make_account_id(201);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(recipient1_id)
        .with_genesis_empty_account(recipient2_id)
        .execute_genesis();

    let err = fixture
        .child_block()
        .with_sau(snark_id, |sau| {
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
        fixture.account_balance(snark_id),
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
