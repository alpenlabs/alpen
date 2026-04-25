//! Deposit-withdraw tests for end-to-end workflows.

use strata_acct_types::BitcoinAmount;
use strata_identifiers::SubjectId;
use strata_ledger_types::ISnarkAccountState;
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;

use crate::{BRIDGE_GATEWAY_ACCT_ID, BRIDGE_GATEWAY_ACCT_SERIAL, test_utils::*};

#[test]
fn test_snark_account_deposit_and_withdrawal() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let deposit_amount = BitcoinAmount::from_sat(150_000_000); // 1.5 BTC; enough to cover withdrawal.
    let dest_subject = SubjectId::from([42u8; 32]);

    let genesis = OLStfFixture::builder();
    let snark_acct_serial = genesis.next_account_serial();
    let genesis_manifest =
        make_deposit_manifest_for_account(1, 1, snark_acct_serial, dest_subject, deposit_amount);

    let mut fixture = genesis
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
}
