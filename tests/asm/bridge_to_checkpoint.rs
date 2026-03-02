//! Bridge <-> Checkpoint subprotocol interaction tests
//!
//! Tests the propagation of deposit events from the bridge subprotocol
//! to the checkpoint subprotocol's available deposit tracking, and the
//! deduction of withdrawals from the available deposit sum.
//!
//! Key interactions tested:
//! - Bridge deposit processing -> checkpoint `available_deposit_sum` increment
//! - Multiple deposits accumulate correctly
//! - Deposit amount matches bridge denomination
//! - Checkpoint with withdrawals deducts from `available_deposit_sum`
//! - Checkpoint rejected when withdrawals exceed available deposits

#![allow(
    unused_crate_dependencies,
    reason = "test dependencies shared across test suite"
)]

use harness::{
    bridge::{create_test_bridge_setup, create_test_checkpoint_setup, BridgeExt},
    test_harness::AsmTestHarnessBuilder,
};
use integration_tests::harness;

/// Verifies that a single bridge deposit updates the checkpoint's available deposit sum.
///
/// Flow:
/// 1. Configure bridge with known operators and denomination
/// 2. Submit a deposit transaction
/// 3. Mine a block so `process_msgs` delivers `DepositProcessed` to checkpoint
/// 4. Verify checkpoint's `available_deposit_sum` equals the deposit denomination
#[tokio::test(flavor = "multi_thread")]
async fn test_deposit_updates_checkpoint_available_sum() {
    let (bridge_params, ctx) = create_test_bridge_setup(3);
    let denomination = ctx.denomination();

    let harness = AsmTestHarnessBuilder::default()
        .with_bridge_config(bridge_params)
        .with_txindex()
        .build()
        .await
        .unwrap();

    // Initialize subprotocols (genesis block creates initial state)
    harness.mine_block(None).await.unwrap();

    // Verify initial state: no deposits tracked yet
    let initial_checkpoint = harness.checkpoint_new_state().unwrap();
    assert_eq!(
        initial_checkpoint.available_deposit_sum(),
        0,
        "Checkpoint should start with zero available deposits"
    );

    let initial_bridge = harness.bridge_state().unwrap();
    assert_eq!(
        initial_bridge.deposits().len(),
        0,
        "Bridge should start with no deposits"
    );

    // Submit a deposit
    harness.submit_deposit(&ctx, 0).await.unwrap();

    // The deposit is processed by bridge in `process_txs`, which emits DepositProcessed.
    // The checkpoint receives DepositProcessed in `process_msgs` of the same block.
    // However, since bridge processes AFTER checkpoint in `process_txs`, the message
    // is delivered in the same block's `process_msgs` phase.
    // Mine one more block to ensure the message has been delivered.
    harness.mine_block(None).await.unwrap();

    // Verify bridge state: deposit should be recorded
    let bridge_state = harness.bridge_state().unwrap();
    assert!(
        bridge_state.deposits().get_deposit(0).is_some(),
        "Bridge should have the deposit"
    );

    // Verify checkpoint state: available_deposit_sum should equal denomination
    let checkpoint_state = harness.checkpoint_new_state().unwrap();
    assert_eq!(
        checkpoint_state.available_deposit_sum(),
        denomination.to_sat(),
        "Checkpoint available_deposit_sum should equal deposit denomination"
    );
}

/// Verifies that multiple deposits accumulate in the checkpoint's available sum.
///
/// Submits 3 deposits and verifies the sum equals 3 * denomination.
#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_deposits_accumulate_in_checkpoint() {
    let (bridge_params, ctx) = create_test_bridge_setup(3);
    let denomination = ctx.denomination();

    let harness = AsmTestHarnessBuilder::default()
        .with_bridge_config(bridge_params)
        .with_txindex()
        .build()
        .await
        .unwrap();

    // Initialize subprotocols
    harness.mine_block(None).await.unwrap();

    let num_deposits = 3u32;
    for i in 0..num_deposits {
        harness.submit_deposit(&ctx, i).await.unwrap();
    }

    // Mine an extra block to ensure all messages are delivered
    harness.mine_block(None).await.unwrap();

    // Verify bridge state
    let bridge_state = harness.bridge_state().unwrap();
    assert_eq!(
        bridge_state.deposits().len(),
        num_deposits,
        "Bridge should have all deposits"
    );

    // Verify checkpoint accumulated sum
    let checkpoint_state = harness.checkpoint_new_state().unwrap();
    let expected_sum = denomination.to_sat() * num_deposits as u64;
    assert_eq!(
        checkpoint_state.available_deposit_sum(),
        expected_sum,
        "Checkpoint available_deposit_sum should equal sum of all deposits"
    );
}

/// Verifies that a checkpoint with withdrawal intents deducts from `available_deposit_sum`.
///
/// Flow:
/// 1. Submit 3 deposits → `available_deposit_sum` = 3 * denomination
/// 2. Submit a valid checkpoint with 1 withdrawal for `denomination` sats
/// 3. Verify `available_deposit_sum` == 2 * denomination (deducted)
/// 4. Verify `verified_tip.epoch` advanced to 1
#[tokio::test(flavor = "multi_thread")]
async fn test_withdrawal_deducts_from_deposit_sum() {
    let genesis_l1_height = AsmTestHarnessBuilder::DEFAULT_GENESIS_HEIGHT as u32;
    let (bridge_params, ctx) = create_test_bridge_setup(3);
    let (checkpoint_params, mut checkpoint_harness) =
        create_test_checkpoint_setup(genesis_l1_height);
    let denomination = ctx.denomination();

    let harness = AsmTestHarnessBuilder::default()
        .with_bridge_config(bridge_params)
        .with_checkpoint_config(checkpoint_params)
        .with_txindex()
        .build()
        .await
        .unwrap();

    // Initialize subprotocols (genesis block)
    harness.mine_block(None).await.unwrap();

    // Submit 3 deposits
    let num_deposits = 3u32;
    for i in 0..num_deposits {
        harness.submit_deposit(&ctx, i).await.unwrap();
    }

    // Mine extra block for message delivery
    harness.mine_block(None).await.unwrap();

    // Verify deposits accumulated
    let checkpoint_state = harness.checkpoint_new_state().unwrap();
    let expected_initial_sum = denomination.to_sat() * num_deposits as u64;
    assert_eq!(
        checkpoint_state.available_deposit_sum(),
        expected_initial_sum,
        "available_deposit_sum should equal 3 * denomination before withdrawal"
    );

    // Submit a checkpoint with 1 withdrawal for denomination sats
    harness
        .submit_checkpoint_with_withdrawals(&mut checkpoint_harness, &[denomination.to_sat()])
        .await
        .unwrap();

    // Verify: available_deposit_sum deducted by withdrawal amount
    let checkpoint_state = harness.checkpoint_new_state().unwrap();
    let expected_after = denomination.to_sat() * (num_deposits as u64 - 1);
    assert_eq!(
        checkpoint_state.available_deposit_sum(),
        expected_after,
        "available_deposit_sum should be deducted by withdrawal amount"
    );

    // Verify: checkpoint epoch advanced
    assert_eq!(
        checkpoint_state.verified_tip().epoch,
        1,
        "verified_tip epoch should advance to 1 after accepted checkpoint"
    );
}

/// Verifies that a checkpoint is rejected when withdrawal intents exceed available deposits.
///
/// Flow:
/// 1. Submit 1 deposit → `available_deposit_sum` = denomination
/// 2. Submit a checkpoint with withdrawals totaling > denomination
/// 3. Verify `available_deposit_sum` unchanged (still == denomination)
/// 4. Verify `verified_tip.epoch` still == 0 (checkpoint was rejected)
#[tokio::test(flavor = "multi_thread")]
async fn test_checkpoint_rejected_when_withdrawals_exceed_deposits() {
    let genesis_l1_height = AsmTestHarnessBuilder::DEFAULT_GENESIS_HEIGHT as u32;
    let (bridge_params, ctx) = create_test_bridge_setup(3);
    let (checkpoint_params, mut checkpoint_harness) =
        create_test_checkpoint_setup(genesis_l1_height);
    let denomination = ctx.denomination();

    let harness = AsmTestHarnessBuilder::default()
        .with_bridge_config(bridge_params)
        .with_checkpoint_config(checkpoint_params)
        .with_txindex()
        .build()
        .await
        .unwrap();

    // Initialize subprotocols (genesis block)
    harness.mine_block(None).await.unwrap();

    // Submit 1 deposit
    harness.submit_deposit(&ctx, 0).await.unwrap();

    // Mine extra block for message delivery
    harness.mine_block(None).await.unwrap();

    // Verify single deposit tracked
    let checkpoint_state = harness.checkpoint_new_state().unwrap();
    assert_eq!(
        checkpoint_state.available_deposit_sum(),
        denomination.to_sat(),
        "available_deposit_sum should equal denomination after 1 deposit"
    );

    // Submit checkpoint with withdrawals exceeding available deposits (2 * denomination > 1 *
    // denomination). The checkpoint should be rejected, so submit_checkpoint_with_withdrawals
    // will still succeed (the tx is mined) but the ASM ignores the invalid checkpoint.
    harness
        .submit_checkpoint_with_withdrawals(
            &mut checkpoint_harness,
            &[denomination.to_sat(), denomination.to_sat()],
        )
        .await
        .unwrap();

    // Verify: available_deposit_sum unchanged
    let checkpoint_state = harness.checkpoint_new_state().unwrap();
    assert_eq!(
        checkpoint_state.available_deposit_sum(),
        denomination.to_sat(),
        "available_deposit_sum should be unchanged when checkpoint is rejected"
    );

    // Verify: epoch did not advance
    assert_eq!(
        checkpoint_state.verified_tip().epoch,
        0,
        "verified_tip epoch should remain 0 when checkpoint is rejected"
    );
}
