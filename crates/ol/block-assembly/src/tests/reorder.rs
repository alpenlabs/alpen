//! Transaction reorder-focused block assembly tests.

use crate::test_utils::{
    MempoolSnarkTxBuilder, TestAccount, TestEnv, TestStorageFixtureBuilder, included_txids,
    template_state_root, test_account_id,
};

async fn build_reorder_env(accounts: impl IntoIterator<Item = TestAccount>) -> TestEnv {
    let env_builder = TestStorageFixtureBuilder::new()
        .with_parent_slot(0)
        .with_accounts(accounts);
    let (fixture, parent_commitment) = env_builder.build_fixture().await;
    TestEnv::from_fixture(fixture, parent_commitment)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_independent_reorder_same_state_root() {
    let account_a = test_account_id(1);
    let account_b = test_account_id(2);
    let receiver_a = test_account_id(3);
    let receiver_b = test_account_id(4);

    let env = build_reorder_env([
        TestAccount::new(account_a, 10_000),
        TestAccount::new(account_b, 10_000),
        TestAccount::new(receiver_a, 0),
        TestAccount::new(receiver_b, 0),
    ])
    .await;

    let tx_a = MempoolSnarkTxBuilder::new(account_a)
        .with_seq_no(0)
        .with_outputs(vec![(receiver_a, 1_000)])
        .build();
    let tx_b = MempoolSnarkTxBuilder::new(account_b)
        .with_seq_no(0)
        .with_outputs(vec![(receiver_b, 2_000)])
        .build();

    let tx_a_id = tx_a.compute_txid();
    let tx_b_id = tx_b.compute_txid();

    let txs_ab = vec![(tx_a_id, tx_a.clone()), (tx_b_id, tx_b.clone())];
    let txs_ba = vec![(tx_b_id, tx_b), (tx_a_id, tx_a)];

    let output_ab = env
        .construct_block(txs_ab)
        .await
        .expect("AB order should construct");

    let output_ba = env
        .construct_block(txs_ba)
        .await
        .expect("BA order should construct");

    let included_ab = included_txids(&output_ab.template);
    let included_ba = included_txids(&output_ba.template);
    assert_eq!(
        included_ab.len(),
        2,
        "AB order should include both transactions"
    );
    assert!(
        included_ab.contains(&tx_a_id) && included_ab.contains(&tx_b_id),
        "AB order should include tx_a and tx_b"
    );
    assert_eq!(
        included_ba.len(),
        2,
        "BA order should include both transactions"
    );
    assert!(
        included_ba.contains(&tx_a_id) && included_ba.contains(&tx_b_id),
        "BA order should include tx_a and tx_b"
    );

    let root_ab = template_state_root(&output_ab.template);
    let root_ba = template_state_root(&output_ba.template);
    assert_eq!(
        root_ab, root_ba,
        "Independent-account transaction reorder should preserve post-state root"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_value_flow_reorder_changes_state_root() {
    let sender = test_account_id(1);
    let relay = test_account_id(2);
    let receiver = test_account_id(3);

    let env = build_reorder_env([
        TestAccount::new(sender, 10_000),
        TestAccount::new(relay, 0),
        TestAccount::new(receiver, 0),
    ])
    .await;

    // tx_fund sends value from sender -> relay.
    let tx_fund = MempoolSnarkTxBuilder::new(sender)
        .with_seq_no(0)
        .with_outputs(vec![(relay, 1_000)])
        .build();
    // tx_spend spends from relay -> receiver, requiring relay to be funded first.
    let tx_spend = MempoolSnarkTxBuilder::new(relay)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, 500)])
        .build();

    let tx_fund_id = tx_fund.compute_txid();
    let tx_spend_id = tx_spend.compute_txid();

    // Cross-account dependency reorder:
    // [fund, spend] keeps both txs eligible, while [spend, fund] makes the
    // first tx ineligible (relay has no funds yet). Different eligibility
    // outcomes must produce a different block/state root.
    let output_fund_then_spend = env
        .construct_block(vec![
            (tx_fund_id, tx_fund.clone()),
            (tx_spend_id, tx_spend.clone()),
        ])
        .await
        .expect("fund->spend order should construct");

    let output_spend_then_fund = env
        .construct_block(vec![(tx_spend_id, tx_spend), (tx_fund_id, tx_fund)])
        .await
        .expect("spend->fund order should construct");

    let included_fund_then_spend = included_txids(&output_fund_then_spend.template);
    let included_spend_then_fund = included_txids(&output_spend_then_fund.template);
    assert_eq!(
        included_fund_then_spend,
        vec![tx_fund_id, tx_spend_id],
        "fund->spend order should include both transactions"
    );
    assert_eq!(
        included_spend_then_fund,
        vec![tx_fund_id],
        "spend->fund order should reject spend and include fund only"
    );

    let root_fund_then_spend = template_state_root(&output_fund_then_spend.template);
    let root_spend_then_fund = template_state_root(&output_spend_then_fund.template);
    assert_ne!(
        root_fund_then_spend, root_spend_then_fund,
        "Cross-account dependency reorder should change post-state root"
    );
}
