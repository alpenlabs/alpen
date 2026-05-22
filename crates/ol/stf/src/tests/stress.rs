//! Stress tests for larger OL STF workloads.
//!
//! Covers large transaction, inbox, and proof batches.

use strata_acct_types::{BitcoinAmount, MessageEntry, MsgPayload};
use strata_ledger_types::ISnarkAccountState;

use crate::{SEQUENCER_ACCT_ID, test_utils::*, verify_block};

/// Representative large workload size for stress tests.
const STRESS_BATCH_SIZE: usize = 1_000;

fn indexed_payload(index: usize) -> Vec<u8> {
    index.to_le_bytes().to_vec()
}

fn msg_payload_from_bytes(data: Vec<u8>) -> MsgPayload {
    MsgPayload::from_bytes(BitcoinAmount::from_sat(0), data)
        .expect("message payload bytes must fit within SSZ max length")
}

#[test]
fn test_stress_inserts_large_inbox_message_batch() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let mut block = fixture.child_block();
    for i in 0..STRESS_BATCH_SIZE {
        block = block.with_gam(snark_acct_id, |gam| gam.with_payload(indexed_payload(i)));
    }
    block.execute();

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        account_state.inbox_mmr().num_entries(),
        STRESS_BATCH_SIZE as u64
    );
    assert_eq!(account_state.next_inbox_msg_idx(), 0);
}

#[test]
fn test_stress_executes_large_tx_batch() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let target_acct_id = make_account_id(TEST_RECIPIENT_ID);
    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .with_genesis_empty_account(target_acct_id)
        .execute_genesis();

    let mut block = fixture.child_block();
    for i in 0..STRESS_BATCH_SIZE {
        block = block.with_gam(target_acct_id, |gam| gam.with_payload(indexed_payload(i)));
    }
    let block = block.execute();

    assert_eq!(
        block
            .completed_block()
            .body()
            .tx_segment()
            .expect("stress block should contain tx segment")
            .txs()
            .len(),
        STRESS_BATCH_SIZE
    );
    assert_eq!(
        fixture.account_balance(target_acct_id),
        BitcoinAmount::from_sat(0)
    );
}

#[test]
fn test_stress_processes_large_inbox_proof_batch() {
    let snark_acct_id = make_account_id(TEST_SNARK_ACCOUNT_ID);
    let mut fixture = OLStfFixture::builder()
        .with_genesis_snark_account(snark_acct_id, |acct| {
            acct.with_balance(BitcoinAmount::from_sat(100_000_000))
        })
        .execute_genesis();

    let payloads = (0..STRESS_BATCH_SIZE)
        .map(indexed_payload)
        .collect::<Vec<_>>();
    let mut gam_block = fixture.child_block();
    for payload in payloads.iter().cloned() {
        gam_block = gam_block.with_gam(snark_acct_id, |gam| gam.with_payload(payload));
    }
    let gam_output = gam_block.execute();

    let mut inbox_tracker = InboxMmrTracker::new();
    let mut processed_msgs = Vec::with_capacity(STRESS_BATCH_SIZE);
    for payload in payloads {
        let msg = MessageEntry::new(
            SEQUENCER_ACCT_ID,
            gam_output.completed_block().header().epoch(),
            msg_payload_from_bytes(payload),
        );
        inbox_tracker.add_message(&msg);
        processed_msgs.push(msg);
    }

    let proofs = (0..STRESS_BATCH_SIZE)
        .map(|i| inbox_tracker.expect_raw_proof_at(i))
        .collect();
    let mut verify_state = fixture.state().clone();
    let parent_header = fixture.parent_header().clone();
    let sau_block = fixture
        .child_block()
        .with_sau(snark_acct_id, |sau| {
            sau.with_processed_messages(processed_msgs, proofs)
                .with_state_root(make_state_root(2))
        })
        .execute();

    verify_block(
        &mut verify_state,
        sau_block.completed_block().header(),
        Some(&parent_header),
        sau_block.completed_block().body(),
    )
    .expect("large-message SAU should verify");

    let account_state = fixture.expect_snark_account(snark_acct_id);
    assert_eq!(
        fixture.account_balance(snark_acct_id),
        BitcoinAmount::from_sat(100_000_000)
    );
    assert_eq!(*account_state.seqno().inner(), 1);
    assert_eq!(account_state.next_inbox_msg_idx(), STRESS_BATCH_SIZE as u64);
}
