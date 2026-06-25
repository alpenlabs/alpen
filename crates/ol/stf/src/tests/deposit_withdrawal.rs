//! Deposit-withdraw tests for end-to-end workflows.

use strata_acct_types::{BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL, BitcoinAmount};
use strata_identifiers::SubjectId;
use strata_ledger_types::{ISnarkAccountState, IStateAccessor};
use strata_msg_fmt::{Msg, OwnedMsg};
use strata_ol_chain_types::SimpleWithdrawalIntentLogData;
use strata_ol_msg_types::DEPOSIT_MSG_TYPE_ID;

use crate::test_utils::*;

#[test]
fn test_snark_account_deposit_and_withdrawal() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let deposit_amount = BitcoinAmount::from_sat(150_000_000); // 1.5 BTC; enough to cover withdrawal.
    let dest_subject = SubjectId::from([42u8; 32]);

    let fixture_builder = OLStfFixture::builder();
    let snark_acct_serial = fixture_builder.next_account_serial();
    let genesis_manifest =
        make_deposit_manifest_for_account(1, 1, snark_acct_serial, dest_subject, deposit_amount);

    let mut fixture = fixture_builder
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_state_root(make_state_root(1))
        })
        .with_genesis_manifest(genesis_manifest)
        .execute_genesis();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        deposit_amount,
        "Account balance should reflect the deposit"
    );

    let account_state_after_genesis = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        account_state_after_genesis.next_inbox_msg_idx(),
        0,
        "Next inbox idx should still be zero (no messages processed yet)"
    );
    assert_eq!(
        account_state_after_genesis.inbox_mmr().num_entries(),
        1,
        "Should have 1 deposit message in inbox after genesis"
    );

    let mut inbox_tracker = InboxMmrTracker::new();
    let deposit_msg = make_deposit_message_entry(0, dest_subject, deposit_amount);
    let deposit_msg_proof = inbox_tracker.add_message(&deposit_msg);

    let withdrawal_amount = BitcoinAmount::from_sat(100_000_000); // Withdraw exactly 1 BTC.
    let withdrawal_dest_desc = b"bc1qexample".to_vec();
    let withdrawal_payload = make_withdrawal_payload(withdrawal_dest_desc.clone());

    let output = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_processed_messages(vec![deposit_msg], vec![deposit_msg_proof])
                .output_message(
                    BRIDGE_GATEWAY_ACCT_ID,
                    withdrawal_amount,
                    withdrawal_payload,
                )
                .with_state_root(make_state_root(2))
                .with_proof(vec![0u8; 32])
        })
        .execute_with_outputs();

    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(50_000_000),
        "Account balance should be reduced by withdrawal amount"
    );

    let withdrawal_log =
        output.expect_typed_log::<SimpleWithdrawalIntentLogData>(BRIDGE_GATEWAY_ACCT_SERIAL);
    assert_eq!(
        withdrawal_log.amt,
        withdrawal_amount.to_sat(),
        "Withdrawal amount should match"
    );
    assert_eq!(
        withdrawal_log.dest.as_slice(),
        withdrawal_dest_desc.as_slice(),
        "Withdrawal destination should match"
    );
    assert_eq!(
        withdrawal_log.selected_operator,
        u32::MAX,
        "Withdrawal operator should preserve the any-operator sentinel"
    );
}

#[test]
fn test_bridge_gateway_direct_transfer_is_silently_dropped() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let limbo_before = fixture.state().limbo_funds();
    let output = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            // Pins the current footgun: funds are deducted from sender but never delivered.
            sau.transfer(BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount::from_sat(10_000_000))
                .with_state_root(make_state_root(2))
        })
        .execute_with_outputs();

    assert_no_bridge_gateway_logs(&output);
    assert_eq!(
        fixture.state().limbo_funds(),
        BitcoinAmount::from_sat(limbo_before.to_sat() + 10_000_000),
        "direct bridge-gateway transfer should sweep value into limbo"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(90_000_000)
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1
    );
}

#[test]
fn test_bridge_gateway_non_denomination_withdrawal_is_silently_dropped() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let limbo_before = fixture.state().limbo_funds();
    let output = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            // Pins the current footgun: funds are deducted from sender but never delivered.
            sau.output_message(
                BRIDGE_GATEWAY_ACCT_ID,
                BitcoinAmount::from_sat(50_000_000),
                make_withdrawal_payload(b"bc1qnondenomination".to_vec()),
            )
            .with_state_root(make_state_root(2))
        })
        .execute_with_outputs();

    assert_no_bridge_gateway_logs(&output);
    assert_eq!(
        fixture.state().limbo_funds(),
        BitcoinAmount::from_sat(limbo_before.to_sat() + 50_000_000),
        "non-denomination withdrawal should sweep value into limbo"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(50_000_000)
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1
    );
}

#[test]
fn test_bridge_gateway_zero_amount_withdrawal_is_silently_dropped() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let limbo_before = fixture.state().limbo_funds();
    let output = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.output_message(
                BRIDGE_GATEWAY_ACCT_ID,
                BitcoinAmount::from_sat(0),
                make_withdrawal_payload(b"bc1qzeroamount".to_vec()),
            )
            .with_state_root(make_state_root(2))
        })
        .execute_with_outputs();

    assert_no_bridge_gateway_logs(&output);
    assert_eq!(
        fixture.state().limbo_funds(),
        limbo_before,
        "zero-amount withdrawal should not change limbo"
    );
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(100_000_000)
    );
    assert_eq!(
        *fixture.expect_snark_account(snark_acct_id).seqno().inner(),
        1
    );
}

#[test]
fn test_bridge_gateway_non_withdrawal_gam_is_silently_dropped() {
    let mut fixture = OLStfFixture::builder().execute_genesis();

    let limbo_before = fixture.state().limbo_funds();
    let output = fixture
        .child_block()
        .with_default_gam(BRIDGE_GATEWAY_ACCT_ID)
        .execute_with_outputs();

    assert_no_bridge_gateway_logs(&output);
    assert_eq!(
        fixture.state().limbo_funds(),
        limbo_before,
        "zero-value non-withdrawal GAM should not change limbo"
    );
    assert!(
        !fixture
            .state()
            .check_account_exists(BRIDGE_GATEWAY_ACCT_ID)
            .expect("account existence check should succeed"),
        "bridge gateway is a special account, not a ledger account"
    );
}

#[test]
fn test_bridge_gateway_wrong_msg_type_dropped() {
    // Properly framed message with a non-withdrawal type byte. The bridge
    // gateway parses the envelope successfully and then drops it because
    // `try_as_withdrawal` returns None, distinct from the empty-payload case
    // which bails at envelope parsing.
    let mut fixture = OLStfFixture::builder().execute_genesis();

    let framed_payload = OwnedMsg::new(DEPOSIT_MSG_TYPE_ID, vec![])
        .expect("valid message framing should construct")
        .to_vec();
    let limbo_before = fixture.state().limbo_funds();
    let output = fixture
        .child_block()
        .with_gam(BRIDGE_GATEWAY_ACCT_ID, |gam| {
            gam.with_payload(framed_payload)
        })
        .execute_with_outputs();

    assert_no_bridge_gateway_logs(&output);
    assert_eq!(
        fixture.state().limbo_funds(),
        limbo_before,
        "zero-value wrong-type bridge-gateway message should not change limbo"
    );
    assert!(
        !fixture
            .state()
            .check_account_exists(BRIDGE_GATEWAY_ACCT_ID)
            .expect("account existence check should succeed"),
        "bridge gateway is a special account, not a ledger account"
    );
}

fn assert_no_bridge_gateway_logs(output: &FixtureBlockOutput) {
    assert!(
        !output.has_log_from_account_serial(BRIDGE_GATEWAY_ACCT_SERIAL),
        "bridge gateway should not emit withdrawal logs"
    );
}
