//! Mempool-facing block assembly failure/report tests.

use std::sync::Arc;

use strata_config::SequencerConfig;
use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
use strata_ol_mempool::{MempoolTxInvalidReason, OLMempoolError};

use crate::{
    BlockAssemblyError, FixedSlotSealing,
    block_assembly::generate_block_template_inner,
    context::BlockAssemblyContext,
    da_tracker::AccumulatedDaData,
    test_utils::{
        FailingStateProvider, MempoolSnarkTxBuilder, MockMempoolFailMode, MockMempoolProvider,
        TEST_SLOTS_PER_EPOCH, TestAccount, TestEnv, TestStorageFixtureBuilder, create_test_storage,
        included_txids, test_account_id,
    },
    types::BlockGenerationConfig,
};

async fn build_mempool_env(accounts: impl IntoIterator<Item = TestAccount>) -> TestEnv {
    let fixture_builder = TestStorageFixtureBuilder::new()
        .with_parent_slot(0)
        .with_accounts(accounts);
    let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
    TestEnv::from_fixture(fixture, parent_commitment)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_missing_parent_fails() {
    let env = build_mempool_env([]).await;

    let missing_parent = OLBlockCommitment::new(999, OLBlockId::from(Buf32::from([7u8; 32])));
    let config = BlockGenerationConfig::new(missing_parent);

    let err = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect_err("missing parent state should fail");

    assert!(
        matches!(err, BlockAssemblyError::ParentStateNotFound(_)),
        "expected ParentStateNotFound(_), got: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_state_provider_failure_propagates() {
    let storage = create_test_storage();
    let mempool = Arc::new(MockMempoolProvider::new());
    let ctx = BlockAssemblyContext::new(storage, mempool, FailingStateProvider, 0);
    let config = BlockGenerationConfig::new(OLBlockCommitment::new(
        1,
        OLBlockId::from(Buf32::from([7; 32])),
    ));
    let epoch_sealing_policy = FixedSlotSealing::new(TEST_SLOTS_PER_EPOCH);
    let sequencer_config = SequencerConfig::default();

    let err = generate_block_template_inner(
        &ctx,
        &epoch_sealing_policy,
        &sequencer_config,
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect_err("state provider error should fail");

    assert!(
        matches!(err, BlockAssemblyError::StateProvider(_)),
        "expected StateProvider(_), got: {err:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_get_transactions_failure_propagates() {
    let env = build_mempool_env([]).await;
    env.mempool()
        .set_fail_mode(MockMempoolFailMode::GetTransactions);
    let config = BlockGenerationConfig::new(env.parent_commitment());

    let err = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect_err("mempool get failure should fail");

    assert!(
        matches!(
            err,
            BlockAssemblyError::Mempool(OLMempoolError::ServiceClosed(_))
        ),
        "expected mempool service-closed error, got: {err:?}"
    );
    assert_eq!(
        env.mempool().report_call_count(),
        0,
        "report_invalid_transactions must not be called when get_transactions fails"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_generation_stage_does_not_report_invalid_txs() {
    let missing_account = test_account_id(9);
    let env = build_mempool_env([]).await;

    // Target an uncreated account so this tx lands in failed_txs.
    let invalid_tx = MempoolSnarkTxBuilder::new(missing_account)
        .with_seq_no(0)
        .build();
    let invalid_txid = invalid_tx.compute_txid();
    env.mempool().add_transaction(invalid_txid, invalid_tx);
    env.mempool()
        .set_fail_mode(MockMempoolFailMode::ReportInvalidTransactions);

    let config = BlockGenerationConfig::new(env.parent_commitment());
    let result = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("inner assembly should not call report_invalid_transactions");
    let (_template, failed_txs, _da) = result.into_parts();
    let expected = vec![(invalid_txid, MempoolTxInvalidReason::Invalid)];
    assert_eq!(
        failed_txs, expected,
        "inner assembly should still return failed_txs payload"
    );
    assert_eq!(
        env.mempool().report_call_count(),
        0,
        "inner assembly should not call report_invalid_transactions"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_no_report_when_all_txs_valid() {
    let account = test_account_id(1);
    let env = build_mempool_env([TestAccount::new(account, 10_000)]).await;

    let valid_tx = MempoolSnarkTxBuilder::new(account).with_seq_no(0).build();
    let valid_txid = valid_tx.compute_txid();
    env.mempool().add_transaction(valid_txid, valid_tx);

    let config = BlockGenerationConfig::new(env.parent_commitment());
    let result = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("valid tx block assembly should succeed");

    let (template, failed_txs, _da) = result.into_parts();
    assert!(
        failed_txs.is_empty(),
        "valid tx path should not produce failed_txs"
    );
    assert_eq!(
        included_txids(&template),
        vec![valid_txid],
        "valid tx should be included in output template"
    );
    assert_eq!(
        env.mempool().report_call_count(),
        0,
        "report_invalid_transactions must not be called when failed_txs is empty"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_exact_failed_txs_payload() {
    let missing_account = test_account_id(11);
    let env = build_mempool_env([]).await;

    let invalid_tx = MempoolSnarkTxBuilder::new(missing_account)
        .with_seq_no(0)
        .build();
    let invalid_txid = invalid_tx.compute_txid();
    env.mempool().add_transaction(invalid_txid, invalid_tx);

    let config = BlockGenerationConfig::new(env.parent_commitment());
    let result = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("assembly should succeed with invalid tx filtered");

    let (_template, failed_txs, _da) = result.into_parts();
    let expected = vec![(invalid_txid, MempoolTxInvalidReason::Invalid)];
    assert_eq!(
        failed_txs, expected,
        "failed_txs should match expected invalid payload"
    );
    assert!(
        env.mempool().last_reported_invalid_txs().is_empty(),
        "inner assembly should not report invalid txs to mempool"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_mixed_failures_keep_order_and_reason() {
    let missing_account = test_account_id(12);
    let low_balance_account = test_account_id(1);
    let receiver = test_account_id(2);
    let env = build_mempool_env([
        TestAccount::new(low_balance_account, 0),
        TestAccount::new(receiver, 0),
    ])
    .await;

    let invalid_tx = MempoolSnarkTxBuilder::new(missing_account)
        .with_seq_no(0)
        .build();
    let invalid_txid = invalid_tx.compute_txid();

    let failed_tx = MempoolSnarkTxBuilder::new(low_balance_account)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, 1)])
        .build();
    let failed_txid = failed_tx.compute_txid();

    env.mempool().add_transaction(invalid_txid, invalid_tx);
    env.mempool().add_transaction(failed_txid, failed_tx);

    let config = BlockGenerationConfig::new(env.parent_commitment());
    let result = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("assembly should succeed with failed tx filtering");

    let (_template, failed_txs, _da) = result.into_parts();
    let expected = vec![
        (invalid_txid, MempoolTxInvalidReason::Invalid),
        (failed_txid, MempoolTxInvalidReason::Failed),
    ];
    assert_eq!(
        failed_txs, expected,
        "failed_txs should match expected invalid payload"
    );
    assert!(
        env.mempool().last_reported_invalid_txs().is_empty(),
        "inner assembly should not report invalid txs to mempool"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_max_txs_returns_only_fetched_failures() {
    let missing_a = test_account_id(13);
    let missing_b = test_account_id(14);
    let env = build_mempool_env([]).await;

    let tx_a = MempoolSnarkTxBuilder::new(missing_a).with_seq_no(0).build();
    let tx_a_id = tx_a.compute_txid();
    let tx_b = MempoolSnarkTxBuilder::new(missing_b).with_seq_no(0).build();
    let tx_b_id = tx_b.compute_txid();
    env.mempool().add_transaction(tx_a_id, tx_a);
    env.mempool().add_transaction(tx_b_id, tx_b);

    let mut sequencer_config = env.sequencer_config().clone();
    sequencer_config.max_txs_per_block = 1;

    let config = BlockGenerationConfig::new(env.parent_commitment());
    let result = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        &sequencer_config,
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("assembly should succeed with limited tx fetch");

    let (_template, failed_txs, _da) = result.into_parts();
    let expected = vec![(tx_a_id, MempoolTxInvalidReason::Invalid)];
    assert_eq!(
        failed_txs, expected,
        "failed_txs should match expected invalid payload"
    );
    assert!(
        env.mempool().last_reported_invalid_txs().is_empty(),
        "inner assembly should not report invalid txs to mempool"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_exec_failure_maps_to_failed() {
    let sender = test_account_id(1);
    let receiver = test_account_id(2);
    let env = build_mempool_env([TestAccount::new(sender, 0), TestAccount::new(receiver, 0)]).await;

    let tx = MempoolSnarkTxBuilder::new(sender)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, 1)])
        .build();
    let txid = tx.compute_txid();
    env.mempool().add_transaction(txid, tx);

    let config = BlockGenerationConfig::new(env.parent_commitment());
    let result = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("assembly should succeed with failed execution filtered");

    let (_template, failed_txs, _da) = result.into_parts();
    let expected = vec![(txid, MempoolTxInvalidReason::Failed)];
    assert_eq!(
        failed_txs, expected,
        "failed_txs should match expected invalid payload"
    );
    assert!(
        env.mempool().last_reported_invalid_txs().is_empty(),
        "inner assembly should not report invalid txs to mempool"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_duplicate_txid_one_fails() {
    let account = test_account_id(1);
    let env = build_mempool_env([TestAccount::new(account, 10_000)]).await;

    let tx = MempoolSnarkTxBuilder::new(account).with_seq_no(0).build();
    let txid = tx.compute_txid();
    env.mempool().add_transaction(txid, tx.clone());
    env.mempool().add_transaction(txid, tx);

    let config = BlockGenerationConfig::new(env.parent_commitment());
    let result = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("assembly should succeed");

    let (_template, failed_txs, _da) = result.into_parts();
    let expected = vec![(txid, MempoolTxInvalidReason::Invalid)];
    assert_eq!(
        failed_txs, expected,
        "failed_txs should match expected invalid payload"
    );
    assert!(
        env.mempool().last_reported_invalid_txs().is_empty(),
        "inner assembly should not report invalid txs to mempool"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_duplicate_txid_both_fail() {
    let missing_account = test_account_id(15);
    let env = build_mempool_env([]).await;

    // Both copies of the same txid fail in the same way (missing account).
    let tx = MempoolSnarkTxBuilder::new(missing_account)
        .with_seq_no(0)
        .build();
    let txid = tx.compute_txid();
    env.mempool().add_transaction(txid, tx.clone());
    env.mempool().add_transaction(txid, tx);

    let config = BlockGenerationConfig::new(env.parent_commitment());
    let result = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect("assembly should succeed with both txs filtered");

    let (_template, failed_txs, _da) = result.into_parts();
    let expected = vec![
        (txid, MempoolTxInvalidReason::Invalid),
        (txid, MempoolTxInvalidReason::Invalid),
    ];
    assert_eq!(
        failed_txs, expected,
        "failed_txs should match expected invalid payload"
    );
    assert!(
        env.mempool().last_reported_invalid_txs().is_empty(),
        "inner assembly should not report invalid txs to mempool"
    );
}
