//! Messaging-focused block assembly tests.

use strata_acct_types::{BitcoinAmount, MessageEntry, MsgPayload};

use crate::test_utils::{
    DEFAULT_ACCOUNT_BALANCE, MempoolSnarkTxBuilder, TestAccount, TestEnv,
    TestStorageFixtureBuilder, account_balance, create_test_message, included_txids,
    snark_account_inbox_len, snark_account_next_inbox_msg_idx, snark_account_seqno,
    test_account_id,
};

async fn build_messaging_env(accounts: impl IntoIterator<Item = TestAccount>) -> TestEnv {
    let env_builder = TestStorageFixtureBuilder::new()
        .with_parent_slot(0)
        .with_accounts(accounts);
    let (fixture, parent_commitment) = env_builder.build_fixture().await;
    TestEnv::from_fixture(fixture, parent_commitment)
}

#[tokio::test(flavor = "multi_thread")]
async fn test_multi_sender_attribution() {
    let sender_a = test_account_id(1);
    let sender_b = test_account_id(2);
    let receiver = test_account_id(3);

    let mut env = build_messaging_env([
        TestAccount::new(sender_a, DEFAULT_ACCOUNT_BALANCE),
        TestAccount::new(sender_b, DEFAULT_ACCOUNT_BALANCE),
        TestAccount::new(receiver, 0),
    ])
    .await;

    let tx_a = MempoolSnarkTxBuilder::new(sender_a)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, 100)])
        .build();
    let tx_a_id = tx_a.compute_txid();

    let tx_b = MempoolSnarkTxBuilder::new(sender_b)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, 250)])
        .build();
    let tx_b_id = tx_b.compute_txid();

    let output_block1 = env
        .construct_block(vec![(tx_a_id, tx_a), (tx_b_id, tx_b)])
        .await
        .expect("block 1 should construct");

    // Receiver update explicitly processes the two expected inbox messages,
    // including source-account attribution. If attribution is wrong, this tx
    // fails during proof generation/execution.
    let epoch = output_block1.template.header().epoch();
    let expected_msg_from_a = MessageEntry::new(
        sender_a,
        epoch,
        MsgPayload::new(BitcoinAmount::from_sat(100), vec![]),
    );
    let expected_msg_from_b = MessageEntry::new(
        sender_b,
        epoch,
        MsgPayload::new(BitcoinAmount::from_sat(250), vec![]),
    );
    let included_block1 = included_txids(&output_block1.template);
    assert_eq!(
        included_block1,
        vec![tx_a_id, tx_b_id],
        "both sender txs should be included in block 1"
    );
    assert!(
        output_block1.failed_txs.is_empty(),
        "no sender tx should fail in block 1"
    );

    // Persist block 1 + post-state as parent for block 2.
    let parent_da_2 = output_block1.accumulated_da.clone();
    let _current_commitment = env.persist(&output_block1).await;

    // Mirror expected entries into storage MMR for block-2 proof generation.
    // If block-1 sender attribution is wrong, block-2 processing below fails
    // because these expected proofs will not match block-1 state MMR.
    env.append_inbox_messages(
        receiver,
        vec![expected_msg_from_a.clone(), expected_msg_from_b.clone()],
    );

    let tx_receiver = MempoolSnarkTxBuilder::new(receiver)
        .with_seq_no(0)
        .with_processed_messages(vec![expected_msg_from_a, expected_msg_from_b])
        .with_new_msg_idx(2)
        .build();
    let tx_receiver_id = tx_receiver.compute_txid();

    let output_block2 = env
        .construct_block_with_da(vec![(tx_receiver_id, tx_receiver)], parent_da_2)
        .await
        .expect("block 2 should construct and process attributed messages");
    let included_block2 = included_txids(&output_block2.template);
    assert_eq!(
        included_block2,
        vec![tx_receiver_id],
        "receiver processing tx should be included in block 2"
    );
    assert!(
        output_block2.failed_txs.is_empty(),
        "receiver processing tx should not fail when sender attribution is correct"
    );

    assert_eq!(
        snark_account_inbox_len(&output_block2.post_state, receiver),
        2,
        "receiver inbox should contain both messages"
    );
    assert_eq!(
        snark_account_next_inbox_msg_idx(&output_block2.post_state, receiver),
        2,
        "receiver processing tx should consume both attributed messages in block 2"
    );
    assert_eq!(
        snark_account_seqno(&output_block2.post_state, receiver),
        1,
        "receiver seqno should advance after processing tx in block 2"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_single_account_inbox_index_progression() {
    let receiver = test_account_id(1);
    let msg0 = create_test_message(2, 1, 100);
    let msg1 = create_test_message(3, 1, 200);
    let inbox_messages = vec![msg0.clone(), msg1.clone()];
    let env = build_messaging_env([
        TestAccount::new(receiver, DEFAULT_ACCOUNT_BALANCE).with_inbox(inbox_messages.clone())
    ])
    .await;

    let tx1 = MempoolSnarkTxBuilder::new(receiver)
        .with_seq_no(0)
        .with_processed_messages(vec![msg0])
        .build();
    let tx1_id = tx1.compute_txid();

    let tx2 = MempoolSnarkTxBuilder::new(receiver)
        .with_seq_no(1)
        .with_processed_messages(vec![msg1])
        .with_new_msg_idx(2)
        .build();
    let tx2_id = tx2.compute_txid();

    let output = env
        .construct_block(vec![(tx1_id, tx1), (tx2_id, tx2)])
        .await
        .expect("block should construct");
    let included = included_txids(&output.template);
    assert_eq!(
        included,
        vec![tx1_id, tx2_id],
        "both processing transactions should be included"
    );
    assert!(
        output.failed_txs.is_empty(),
        "no processing transaction should fail"
    );

    assert_eq!(
        snark_account_next_inbox_msg_idx(&output.post_state, receiver),
        2,
        "sequential processing should advance next_inbox_msg_idx to 2"
    );
    assert_eq!(
        snark_account_seqno(&output.post_state, receiver),
        2,
        "receiver seqno should advance across both updates"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_message_value_flow_balance_updates() {
    let sender = test_account_id(1);
    let receiver = test_account_id(2);

    let env = build_messaging_env([
        TestAccount::new(sender, 1_000),
        TestAccount::new(receiver, 0),
    ])
    .await;

    let tx = MempoolSnarkTxBuilder::new(sender)
        .with_seq_no(0)
        .with_outputs(vec![(receiver, 400)])
        .build();
    let tx_id = tx.compute_txid();

    let output = env
        .construct_block(vec![(tx_id, tx)])
        .await
        .expect("block should construct");

    let included = included_txids(&output.template);
    assert_eq!(included, vec![tx_id], "message-value tx should be included");
    assert!(
        output.failed_txs.is_empty(),
        "message-value tx should not fail"
    );

    assert_eq!(
        account_balance(&output.post_state, sender),
        BitcoinAmount::from_sat(600),
        "sender balance should be debited by message value"
    );
    assert_eq!(
        account_balance(&output.post_state, receiver),
        BitcoinAmount::from_sat(400),
        "receiver balance should be credited by message value"
    );
    assert_eq!(
        snark_account_inbox_len(&output.post_state, receiver),
        1,
        "receiver inbox should contain the delivered message"
    );
}
