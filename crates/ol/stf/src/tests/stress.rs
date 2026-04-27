//! Ignored stress tests for larger OL STF workloads.

use strata_acct_types::{BitcoinAmount, MessageEntry, MsgPayload};
use strata_ledger_types::{IAccountState, ISnarkAccountState, IStateAccessor};

use crate::{
    SEQUENCER_ACCT_ID, assembly::BlockComponents, context::BlockInfo, test_utils::*, verify_block,
};

/// Representative large workload size for stress tests.
const STRESS_BATCH_SIZE: usize = 1_000;

fn indexed_payload(index: usize) -> Vec<u8> {
    index.to_le_bytes().to_vec()
}

#[test]
#[ignore = "stress test"]
fn test_stress_inserts_large_inbox_message_batch() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    let txs = (0..STRESS_BATCH_SIZE)
        .map(|i| make_gam_tx_with_payload(snark_id, indexed_payload(i)))
        .collect();
    let block_info = BlockInfo::new(1_001_000, 1, 1);

    execute_block(
        &mut state,
        &block_info,
        Some(genesis_block.header()),
        BlockComponents::new_txs_from_ol_transactions(txs),
    )
    .expect("large GAM inbox insert batch should execute");

    let (_, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        snark_account_state.inbox_mmr().num_entries(),
        STRESS_BATCH_SIZE as u64
    );
    assert_eq!(snark_account_state.next_inbox_msg_idx(), 0);
}

#[test]
#[ignore = "stress test"]
fn test_stress_executes_large_tx_batch() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let target_id = get_test_recipient_account_id();
    create_empty_account(&mut state, target_id);
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    let txs = (0..STRESS_BATCH_SIZE)
        .map(|i| make_gam_tx_with_payload(target_id, indexed_payload(i)))
        .collect();
    let block_info = BlockInfo::new(1_001_000, 1, 1);
    let block = execute_block(
        &mut state,
        &block_info,
        Some(genesis_block.header()),
        BlockComponents::new_txs_from_ol_transactions(txs),
    )
    .expect("large transaction batch should execute");

    assert_eq!(
        block
            .body()
            .tx_segment()
            .expect("stress block should contain tx segment")
            .txs()
            .len(),
        STRESS_BATCH_SIZE
    );
    let target_account = state.get_account_state(target_id).unwrap().unwrap();
    assert_eq!(target_account.balance(), BitcoinAmount::zero());
}

#[test]
#[ignore = "stress test"]
fn test_stress_processes_large_inbox_proof_batch() {
    let mut state = create_test_genesis_state();
    let snark_id = get_test_snark_account_id();
    let genesis_block = setup_genesis_with_snark_account(&mut state, snark_id, 100_000_000);

    let mut inbox_tracker = InboxMmrTracker::new();
    let mut processed_msgs = Vec::with_capacity(STRESS_BATCH_SIZE);
    let txs: Vec<_> = (0..STRESS_BATCH_SIZE)
        .map(|i| {
            let payload = indexed_payload(i);
            let msg = MessageEntry::new(
                SEQUENCER_ACCT_ID,
                1,
                MsgPayload::new(BitcoinAmount::zero(), payload.clone()),
            );
            inbox_tracker.add_message(&msg);
            processed_msgs.push(msg);
            make_gam_tx_with_payload(snark_id, payload)
        })
        .collect();

    let gam_block_info = BlockInfo::new(1_001_000, 1, 1);
    let gam_block = execute_block(
        &mut state,
        &gam_block_info,
        Some(genesis_block.header()),
        BlockComponents::new_txs_from_ol_transactions(txs),
    )
    .expect("large GAM inbox insert batch should execute");

    let proofs = (0..STRESS_BATCH_SIZE)
        .map(|i| inbox_tracker.raw_proof(i))
        .collect();
    let snark_account_state = lookup_snark_state(&state, snark_id);
    let update_tx = SnarkUpdateBuilder::from_snark_state(snark_account_state.clone())
        .with_processed_msgs(processed_msgs)
        .with_inbox_proofs(proofs)
        .build(snark_id, get_test_state_root(2), get_test_proof(1));

    let mut verify_state = state.clone();
    let sau_block = execute_tx_in_block(&mut state, gam_block.header(), update_tx, 2, 1)
        .expect("large-message SAU should execute");

    verify_block(
        &mut verify_state,
        sau_block.header(),
        Some(gam_block.header()),
        sau_block.body(),
    )
    .expect("large-message SAU should verify");

    let (ol_account_state, snark_account_state) = lookup_snark_account_states(&state, snark_id);
    assert_eq!(
        ol_account_state.balance(),
        BitcoinAmount::from_sat(100_000_000)
    );
    assert_eq!(*snark_account_state.seqno().inner(), 1);
    assert_eq!(
        snark_account_state.next_inbox_msg_idx(),
        STRESS_BATCH_SIZE as u64
    );
}
