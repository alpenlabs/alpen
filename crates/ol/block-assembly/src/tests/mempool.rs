//! Mempool-facing block assembly failure/report tests.

use std::sync::Arc;

use strata_config::SequencerConfig;
use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
use strata_ol_mempool::OLMempoolError;

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
async fn test_report_failure_propagates() {
    let missing_account = test_account_id(9);
    let env = build_mempool_env([]).await;

    // Target an uncreated account so this tx lands in failed_txs and triggers reporting.
    let invalid_tx = MempoolSnarkTxBuilder::new(missing_account)
        .with_seq_no(0)
        .build();
    let invalid_txid = invalid_tx.compute_txid();
    env.mempool().add_transaction(invalid_txid, invalid_tx);
    env.mempool()
        .set_fail_mode(MockMempoolFailMode::ReportInvalidTransactions);

    let config = BlockGenerationConfig::new(env.parent_commitment());
    let err = generate_block_template_inner(
        env.ctx(),
        env.epoch_sealing_policy(),
        env.sequencer_config(),
        config,
        AccumulatedDaData::new_empty(),
    )
    .await
    .expect_err("report_invalid_transactions failure should fail");

    assert!(
        matches!(
            err,
            BlockAssemblyError::Mempool(OLMempoolError::ServiceClosed(_))
        ),
        "expected mempool service-closed error, got: {err:?}"
    );
    assert_eq!(
        env.mempool().report_call_count(),
        1,
        "failed tx report should be attempted exactly once"
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
