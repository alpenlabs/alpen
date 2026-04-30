//! Post-state effects and rollback-focused block assembly tests.

use strata_acct_types::{AccountSerial, BitcoinAmount};
use strata_asm_proto_checkpoint_types::MAX_OL_LOGS_PER_CHECKPOINT;
use strata_ol_chain_types_new::OLLog;
use strata_ol_mempool::MempoolTxInvalidReason;
use strata_ol_state_support_types::EpochDaAccumulator;

use crate::{
    da_tracker::AccumulatedDaData,
    test_utils::{
        DEFAULT_ACCOUNT_BALANCE, MempoolSnarkTxBuilder, TestAccount, TestEnv,
        TestStorageFixtureBuilder, account_balance, included_txids, test_account_id,
        withdrawal_intents,
    },
};

async fn build_effects_env(accounts: impl IntoIterator<Item = TestAccount>) -> TestEnv {
    let env_builder = accounts.into_iter().fold(
        TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3),
        |builder, account| builder.with_account(account),
    );
    let (fixture, parent_commitment) = env_builder.build_fixture().await;
    TestEnv::from_fixture(fixture, parent_commitment)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_transfer_balances_update() {
    let sender = test_account_id(1);
    let receiver = test_account_id(2);
    let transfer_sats = 7_000u64;

    let env = build_effects_env([
        TestAccount::new(sender, DEFAULT_ACCOUNT_BALANCE),
        TestAccount::new(receiver, 0),
    ])
    .await;

    let tx = MempoolSnarkTxBuilder::new(sender)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, transfer_sats)])
        .build();
    let txid = tx.compute_txid();

    let output = env
        .construct_block(vec![(txid, tx)])
        .await
        .expect("transfer block should assemble");
    assert!(output.failed_txs.is_empty(), "transfer should succeed");

    let included = included_txids(&output.template);
    assert_eq!(included.len(), 1, "block should include one transfer");

    assert_eq!(
        account_balance(&output.post_state, sender),
        BitcoinAmount::from_sat(DEFAULT_ACCOUNT_BALANCE - transfer_sats),
        "sender balance should be debited by transfer amount"
    );
    assert_eq!(
        account_balance(&output.post_state, receiver),
        BitcoinAmount::from_sat(transfer_sats),
        "receiver balance should be credited by transfer amount"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_withdrawal_log_content() {
    let sender = test_account_id(3);
    let withdrawal_sats = 100_000_000u64;
    let withdrawal_dest = b"bc1qeffectswithdrawal".to_vec();

    let env = build_effects_env([TestAccount::new(sender, DEFAULT_ACCOUNT_BALANCE)]).await;

    let tx = MempoolSnarkTxBuilder::new(sender)
        .with_seq_no(0)
        .with_withdrawal(withdrawal_sats, withdrawal_dest.clone())
        .build();
    let txid = tx.compute_txid();

    let output = env
        .construct_block(vec![(txid, tx)])
        .await
        .expect("withdrawal block should assemble");
    assert!(output.failed_txs.is_empty(), "withdrawal tx should succeed");

    let decoded_withdrawal_logs = withdrawal_intents(&output);
    assert_eq!(
        decoded_withdrawal_logs.len(),
        1,
        "expected exactly one decodable withdrawal intent log"
    );

    let withdrawal_log = &decoded_withdrawal_logs[0];
    assert_eq!(
        withdrawal_log.amt, withdrawal_sats,
        "withdrawal amount should match"
    );
    assert_eq!(
        withdrawal_log.dest.as_slice(),
        withdrawal_dest.as_slice(),
        "withdrawal destination should match"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rollback_preserves_first_tx_effects() {
    let sender = test_account_id(4);
    let receiver = test_account_id(5);
    let first_transfer_sats = 5_000u64;

    let env = build_effects_env([
        TestAccount::new(sender, DEFAULT_ACCOUNT_BALANCE),
        TestAccount::new(receiver, 0),
    ])
    .await;

    let tx1 = MempoolSnarkTxBuilder::new(sender)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, first_transfer_sats)])
        .build();
    let tx1_id = tx1.compute_txid();

    // Force deterministic failure via seq gap: account seq_no should be 1 after tx1.
    let tx2 = MempoolSnarkTxBuilder::new(sender)
        .with_seq_no(2)
        .with_outputs(vec![(receiver, 1_000)])
        .build();
    let tx2_id = tx2.compute_txid();

    let output = env
        .construct_block(vec![(tx1_id, tx1), (tx2_id, tx2)])
        .await
        .expect("rollback block should assemble");

    let included = included_txids(&output.template);
    assert_eq!(included.len(), 1, "only first tx should be included");
    assert_eq!(output.failed_txs.len(), 1, "second tx should fail");
    assert_eq!(
        output.failed_txs[0].0, tx2_id,
        "failed entry should correspond to the second tx"
    );

    assert_eq!(
        account_balance(&output.post_state, sender),
        BitcoinAmount::from_sat(DEFAULT_ACCOUNT_BALANCE - first_transfer_sats),
        "sender should reflect only first tx debit"
    );
    assert_eq!(
        account_balance(&output.post_state, receiver),
        BitcoinAmount::from_sat(first_transfer_sats),
        "receiver should reflect only first tx credit"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_hard_limit_rollback_discards_tx2_log() {
    let account = test_account_id(6);
    let receiver = test_account_id(7);
    let withdrawal_sats = 100_000_000u64;

    let env = build_effects_env([
        TestAccount::new(account, DEFAULT_ACCOUNT_BALANCE),
        TestAccount::new(receiver, 0),
    ])
    .await;

    let tx1 = MempoolSnarkTxBuilder::new(account)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, 1_000)])
        .build();
    let tx1_id = tx1.compute_txid();

    // This tx would emit a bridge-gateway withdrawal log if committed.
    let tx2 = MempoolSnarkTxBuilder::new(account)
        .with_seq_no(1)
        .with_withdrawal(withdrawal_sats, b"bc1qhardlimitrollback".to_vec())
        .build();
    let tx2_id = tx2.compute_txid();

    let seeded_count = (MAX_OL_LOGS_PER_CHECKPOINT as usize) - 1;
    // Offset seeded log source serials away from test account serials (1..10)
    // so seeded checkpoint logs cannot collide with real tx-emitted sources.
    let seeded_logs: Vec<_> = (0..seeded_count)
        .map(|i| OLLog::new(AccountSerial::from((1_000 + i) as u32), vec![]))
        .collect();
    let parent_da = AccumulatedDaData::new(EpochDaAccumulator::default(), seeded_logs);

    let output = env
        .construct_block_with_da(vec![(tx1_id, tx1), (tx2_id, tx2)], parent_da)
        .await
        .expect("hard-limit rollback scenario should assemble");

    let included = included_txids(&output.template);
    assert_eq!(
        included.len(),
        1,
        "tx1 should be kept, tx2 should be rolled back"
    );
    assert!(
        output.failed_txs.is_empty(),
        "hard-limit rollback should defer/stop, not mark tx invalid"
    );

    assert_eq!(
        output.accumulated_da.logs().len(),
        seeded_count,
        "rolled-back tx2 log must not be appended to accumulated DA logs"
    );
    assert_eq!(
        withdrawal_intents(&output).len(),
        0,
        "rolled-back tx2 must not leave bridge-gateway withdrawal logs"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_rollback_on_insufficient_balance_preserves_prior_effects() {
    let rich = test_account_id(8);
    let poor = test_account_id(9);
    let receiver = test_account_id(10);
    let tx1_amount = 5_000u64;
    let tx2_amount = 1_000u64;

    let env = build_effects_env([
        TestAccount::new(rich, DEFAULT_ACCOUNT_BALANCE),
        TestAccount::new(poor, 0),
        TestAccount::new(receiver, 0),
    ])
    .await;

    let tx1 = MempoolSnarkTxBuilder::new(rich)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, tx1_amount)])
        .build();
    let tx1_id = tx1.compute_txid();

    // Non-seq-gap deterministic failure path: insufficient balance.
    let tx2 = MempoolSnarkTxBuilder::new(poor)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, tx2_amount)])
        .build();
    let tx2_id = tx2.compute_txid();

    let output = env
        .construct_block(vec![(tx1_id, tx1), (tx2_id, tx2)])
        .await
        .expect("mixed success/failure block should assemble");

    let included = included_txids(&output.template);
    assert_eq!(included.len(), 1, "only tx1 should be included");
    assert_eq!(output.failed_txs.len(), 1, "tx2 should fail");
    assert_eq!(output.failed_txs[0].0, tx2_id, "failed tx should be tx2");
    assert_eq!(
        output.failed_txs[0].1,
        MempoolTxInvalidReason::Failed,
        "insufficient-balance path should map to Failed"
    );

    assert_eq!(
        account_balance(&output.post_state, rich),
        BitcoinAmount::from_sat(DEFAULT_ACCOUNT_BALANCE - tx1_amount),
        "rich account should reflect only tx1 debit"
    );
    assert_eq!(
        account_balance(&output.post_state, receiver),
        BitcoinAmount::from_sat(tx1_amount),
        "receiver should reflect only tx1 credit"
    );
    assert_eq!(
        account_balance(&output.post_state, poor),
        BitcoinAmount::from_sat(0),
        "failing tx2 should not mutate poor account balance"
    );
}
