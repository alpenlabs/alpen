//! Tests for SAU log emission.

use strata_acct_types::{BitcoinAmount, MessageEntry, MsgPayload};
use strata_ledger_types::ISnarkAccountState;
use strata_ol_chain_types_new::SnarkAccountUpdateLogData;

use crate::{SEQUENCER_ACCT_ID, test_utils::*};

#[test]

fn test_snark_update_emits_log() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let snark_acct_serial = fixture.account_serial(snark_acct_id);
    let pre_msg_idx = fixture
        .expect_snark_account(snark_acct_id)
        .next_inbox_msg_idx();
    let extra_data = b"snark-update-extra".to_vec();

    let output = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_extra_data(extra_data.clone())
                .with_state_root(make_state_root(2))
        })
        .execute_with_outputs();

    let log = output.expect_typed_log::<SnarkAccountUpdateLogData>(snark_acct_serial);
    assert_eq!(
        log.new_msg_idx, pre_msg_idx,
        "no messages processed; new_msg_idx should equal pre-update next_inbox_msg_idx"
    );
    assert_eq!(log.extra_data.as_ref(), extra_data.as_slice());
}

#[test]
fn test_snark_update_emits_log_with_processed_message() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);

    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    // Block 1: deliver a GAM message into the snark account's inbox.
    let mut inbox_tracker = InboxMmrTracker::new();
    let gam_output = fixture
        .child_block()
        .with_default_gam(snark_acct_id)
        .execute();

    // Mirror the message into the parallel MMR so we can produce a valid proof.
    let epoch = gam_output.completed_block().header().epoch();
    let gam_msg = MessageEntry::new(SEQUENCER_ACCT_ID, epoch, MsgPayload::new_empty());
    let gam_proof = inbox_tracker.add_message(&gam_msg);

    // Capture pre-update state.
    let snark_acct_serial = fixture.account_serial(snark_acct_id);
    let pre_msg_idx = fixture
        .expect_snark_account(snark_acct_id)
        .next_inbox_msg_idx();

    // Block 2: snark update that processes the inbox message.
    let output = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_processed_messages(vec![gam_msg], vec![gam_proof])
                .with_state_root(make_state_root(2))
        })
        .execute_with_outputs();

    let log = output.expect_typed_log::<SnarkAccountUpdateLogData>(snark_acct_serial);
    assert_eq!(
        log.new_msg_idx,
        pre_msg_idx + 1,
        "new_msg_idx should advance by the number of processed messages"
    );
    assert!(
        log.extra_data.is_empty(),
        "extra_data was not set on this update"
    );
}
