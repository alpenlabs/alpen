//! Admin → Checkpoint subprotocol interaction tests
//!
//! Tests the propagation of admin updates to the checkpoint subprotocol.
//!
//! Key interactions tested:
//! - Sequencer key updates → checkpoint cred_rule
//! - Predicate updates → checkpoint predicate (after activation)

#![allow(
    unused_crate_dependencies,
    reason = "test dependencies shared across test suite"
)]

use harness::{
    admin::{create_test_admin_setup, predicate_update, sequencer_update, AdminExt},
    checkpoint::CheckpointExt,
    test_harness::AsmTestHarnessBuilder,
};
use integration_tests::harness;
use strata_asm_txs_admin::actions::updates::predicate::ProofType;
use strata_predicate::PredicateKey;
use strata_primitives::block_credential::CredRule;

// ============================================================================
// Sequencer Key → Checkpoint Cred Rule
// ============================================================================

/// Verifies sequencer key updates propagate to checkpoint subprotocol.
#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_update_propagates_to_checkpoint() {
    let (admin_params, mut ctx) = create_test_admin_setup(2);
    let harness = AsmTestHarnessBuilder::default()
        .with_admin_params(admin_params)
        .build()
        .await
        .unwrap();

    // Initialize subprotocols (genesis state has no sections)
    harness.mine_block(None).await.unwrap();

    let initial_checkpoint_state = harness.checkpoint_state().unwrap();
    let initial_cred_rule = initial_checkpoint_state.cred_rule.clone();

    // Submit a sequencer key update
    let new_key = [42u8; 32];
    harness
        .submit_admin_action(&mut ctx, sequencer_update(new_key))
        .await
        .unwrap();

    let final_checkpoint_state = harness.checkpoint_state().unwrap();

    assert_ne!(
        final_checkpoint_state.cred_rule, initial_cred_rule,
        "Checkpoint cred_rule should be updated after sequencer key change"
    );

    // Verify it's specifically a SchnorrKey with our new key
    match &final_checkpoint_state.cred_rule {
        CredRule::SchnorrKey(key) => {
            assert_eq!(
                key.as_ref(),
                &new_key,
                "Checkpoint should have the new sequencer key"
            );
        }
        other => panic!(
            "Expected SchnorrKey cred_rule after sequencer update, got {:?}",
            other
        ),
    }
}

/// Verifies multiple sequential sequencer key updates result in checkpoint having the latest key.
#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_sequencer_updates_checkpoint_has_latest() {
    let (admin_params, mut ctx) = create_test_admin_setup(2);
    let harness = AsmTestHarnessBuilder::default()
        .with_admin_params(admin_params)
        .build()
        .await
        .unwrap();

    // Initialize subprotocols
    harness.mine_block(None).await.unwrap();

    // Submit 3 sequencer key updates in sequence
    let key1 = [1u8; 32];
    let key2 = [2u8; 32];
    let key3 = [3u8; 32];

    harness
        .submit_admin_action(&mut ctx, sequencer_update(key1))
        .await
        .unwrap();
    harness
        .submit_admin_action(&mut ctx, sequencer_update(key2))
        .await
        .unwrap();
    harness
        .submit_admin_action(&mut ctx, sequencer_update(key3))
        .await
        .unwrap();

    // Checkpoint should have the latest key (key3)
    let checkpoint_state = harness.checkpoint_state().unwrap();
    match &checkpoint_state.cred_rule {
        CredRule::SchnorrKey(key) => {
            assert_eq!(
                key.as_ref(),
                &key3,
                "Checkpoint should have the latest sequencer key"
            );
        }
        other => panic!("Expected SchnorrKey cred_rule, got {:?}", other),
    }

    // All 3 updates should have been processed
    let state = harness.admin_state().unwrap();
    assert_eq!(
        state.next_update_id(),
        3,
        "All 3 updates should be processed"
    );
}

// ============================================================================
// Predicate Update → Checkpoint Predicate
// ============================================================================

/// Verifies predicate (verifying key) updates propagate to checkpoint after activation.
///
/// Flow:
/// 1. Submit predicate update (gets queued)
/// 2. Mine blocks to trigger activation (confirmation_depth=2)
/// 3. Verify checkpoint's predicate field is updated
#[tokio::test(flavor = "multi_thread")]
async fn test_predicate_update_propagates_to_checkpoint() {
    let (admin_params, mut ctx) = create_test_admin_setup(2);
    let harness = AsmTestHarnessBuilder::default()
        .with_admin_params(admin_params)
        .build()
        .await
        .unwrap();

    // Initialize subprotocols
    harness.mine_block(None).await.unwrap();

    let initial_checkpoint_state = harness.checkpoint_state().unwrap();
    let initial_predicate = initial_checkpoint_state.predicate.clone();

    // Submit a predicate update (gets queued for StrataAdministrator role)
    let new_predicate = PredicateKey::always_accept();
    harness
        .submit_admin_action(
            &mut ctx,
            predicate_update(new_predicate.clone(), ProofType::OLStf),
        )
        .await
        .unwrap();

    // Verify it's queued, not applied yet
    let state = harness.admin_state().unwrap();
    assert_eq!(state.queued().len(), 1, "Predicate update should be queued");

    // Checkpoint predicate should be unchanged while update is queued
    let checkpoint_state = harness.checkpoint_state().unwrap();
    assert_eq!(
        checkpoint_state.predicate, initial_predicate,
        "Checkpoint predicate should not change while update is queued"
    );

    // Mine blocks to trigger activation (confirmation_depth=2)
    harness.mine_block(None).await.unwrap();
    harness.mine_block(None).await.unwrap();

    // Now verify checkpoint's predicate has been updated
    let final_checkpoint_state = harness.checkpoint_state().unwrap();
    assert_eq!(
        final_checkpoint_state.predicate, new_predicate,
        "Checkpoint predicate should be updated after activation"
    );

    // And admin queue should be empty
    let final_state = harness.admin_state().unwrap();
    assert_eq!(
        final_state.queued().len(),
        0,
        "Queue should be empty after activation"
    );
}

// ============================================================================
// Combined Updates
// ============================================================================

/// Verifies sequencer key update followed by predicate update both affect checkpoint.
///
/// Tests the interaction between immediate updates (sequencer) and queued updates (predicate).
#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_and_predicate_updates_both_apply() {
    let (admin_params, mut ctx) = create_test_admin_setup(2);
    let harness = AsmTestHarnessBuilder::default()
        .with_admin_params(admin_params)
        .build()
        .await
        .unwrap();

    // Initialize subprotocols
    harness.mine_block(None).await.unwrap();

    let initial_checkpoint_state = harness.checkpoint_state().unwrap();

    // Submit sequencer update (applies immediately)
    let new_sequencer_key = [99u8; 32];
    harness
        .submit_admin_action(&mut ctx, sequencer_update(new_sequencer_key))
        .await
        .unwrap();

    // Checkpoint should already have new sequencer key
    let mid_checkpoint_state = harness.checkpoint_state().unwrap();
    match &mid_checkpoint_state.cred_rule {
        CredRule::SchnorrKey(key) => {
            assert_eq!(
                key.as_ref(),
                &new_sequencer_key,
                "Sequencer key should be updated immediately"
            );
        }
        other => panic!("Expected SchnorrKey cred_rule, got {:?}", other),
    }

    // Submit predicate update (gets queued with activation_height = current + confirmation_depth)
    let new_predicate = PredicateKey::always_accept();
    harness
        .submit_admin_action(
            &mut ctx,
            predicate_update(new_predicate.clone(), ProofType::OLStf),
        )
        .await
        .unwrap();

    // Predicate should still be initial (update is queued)
    let checkpoint_state = harness.checkpoint_state().unwrap();
    assert_eq!(
        checkpoint_state.predicate, initial_checkpoint_state.predicate,
        "Predicate should not change yet (update is queued)"
    );

    // Admin should have the update queued
    let admin_state = harness.admin_state().unwrap();
    assert_eq!(
        admin_state.queued().len(),
        1,
        "Predicate update should be queued"
    );

    // Mine blocks to trigger activation (confirmation_depth=2)
    harness.mine_block(None).await.unwrap();
    harness.mine_block(None).await.unwrap();

    // Admin queue should be empty (update activated)
    let admin_state = harness.admin_state().unwrap();
    assert_eq!(
        admin_state.queued().len(),
        0,
        "Queue should be empty after activation"
    );

    // Now both should be updated in checkpoint
    let final_checkpoint_state = harness.checkpoint_state().unwrap();
    match &final_checkpoint_state.cred_rule {
        CredRule::SchnorrKey(key) => {
            assert_eq!(
                key.as_ref(),
                &new_sequencer_key,
                "Sequencer key should still be the new value"
            );
        }
        other => panic!("Expected SchnorrKey cred_rule, got {:?}", other),
    }
    assert_eq!(
        final_checkpoint_state.predicate, new_predicate,
        "Predicate should now be updated after activation"
    );
}
