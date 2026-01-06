//! Admin subprotocol integration tests
//!
//! Tests the admin subprotocol's ability to process governance transactions.
//!
//! # Ergonomic API
//!
//! These tests use the harness's ergonomic admin API:
//! ```ignore
//! let harness = create_test_harness().await?;
//! let mut ctx = harness.admin_context();
//! harness.submit_admin_action(&mut ctx, sequencer_update([1u8; 32])).await?;
//! let state = harness.admin_state()?;
//! ```

#![allow(
    unused_crate_dependencies,
    reason = "test dependencies shared across test suite"
)]

use std::{num::NonZero, time::Duration};

use bitcoin::secp256k1::{PublicKey, Secp256k1, SecretKey};
use bitcoind_async_client::traits::Reader;
use common::{
    admin::{
        cancel_update, multisig_config_update, operator_set_update, predicate_update,
        sequencer_update, AdminExt, SUBPROTOCOL_ID as ADMIN_SUBPROTOCOL_ID,
    },
    checkpoint::CheckpointExt,
    harness::{create_test_harness, AsmTestHarness},
};
use integration_tests::common;
use rand::rngs::OsRng;
use strata_asm_txs_admin::{parser::SignedPayload, test_utils::create_signature_set};
use strata_crypto::threshold_signature::{
    CompressedPublicKey, IndexedSignature, SignatureSet, ThresholdConfig,
};
use strata_l1_txfmt::ParseConfig;
use strata_predicate::PredicateKey;
use strata_primitives::{
    block_credential::CredRule,
    roles::{ProofType, Role},
};

// ============================================================================
// Sequencer Updates (StrataSequencerManager role - applied immediately)
// ============================================================================

/// Verifies sequencer updates are applied immediately (not queued).
#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_update_applies_immediately() {
    let harness = create_test_harness().await.unwrap();
    let mut ctx = harness.admin_context();

    harness
        .submit_admin_action(&mut ctx, sequencer_update([1u8; 32]))
        .await
        .unwrap();

    let state = harness.admin_state().unwrap();

    assert_eq!(
        state.queued().len(),
        0,
        "Sequencer update should apply immediately, not be queued"
    );
    assert_eq!(
        state.next_update_id(),
        1,
        "Update ID should increment for all updates"
    );
}

/// Verifies sequencer key updates propagate to checkpoint subprotocol.
#[tokio::test(flavor = "multi_thread")]
async fn test_sequencer_update_propagates_to_checkpoint() {
    let harness = create_test_harness().await.unwrap();
    let mut ctx = harness.admin_context();

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

// ============================================================================
// Queued Updates (StrataAdministrator role - require confirmation depth)
// ============================================================================

/// Verifies operator set updates are queued (not applied immediately).
#[tokio::test(flavor = "multi_thread")]
async fn test_operator_update_is_queued() {
    let harness = create_test_harness().await.unwrap();
    let mut ctx = harness.admin_context();

    harness
        .submit_admin_action(
            &mut ctx,
            operator_set_update(vec![[10u8; 32], [11u8; 32]], vec![[20u8; 32]]),
        )
        .await
        .unwrap();

    let state = harness.admin_state().unwrap();

    assert_eq!(
        state.queued().len(),
        1,
        "Operator set update should be queued"
    );
    assert_eq!(
        state.next_update_id(),
        1,
        "Update ID should increment after queuing"
    );

    let queued = &state.queued()[0];
    assert_eq!(*queued.id(), 0, "First queued update should have ID 0");
}

/// Verifies multisig config updates are queued (not applied immediately).
#[tokio::test(flavor = "multi_thread")]
async fn test_multisig_update_is_queued() {
    let harness = create_test_harness().await.unwrap();
    let mut ctx = harness.admin_context();

    // Initialize subprotocols
    harness.mine_block(None).await.unwrap();

    let initial_state = harness.admin_state().unwrap();
    let initial_auth = initial_state
        .authority(Role::StrataAdministrator)
        .expect("Admin authority should exist");
    let initial_member_count = initial_auth.config().keys().len();

    // Generate a new public key to add
    let secp = Secp256k1::new();
    let new_privkey = SecretKey::new(&mut OsRng);
    let new_pubkey = PublicKey::from_secret_key(&secp, &new_privkey);
    let new_member = CompressedPublicKey::from(new_pubkey);

    harness
        .submit_admin_action(
            &mut ctx,
            multisig_config_update(Role::StrataAdministrator, vec![new_member], vec![], 1),
        )
        .await
        .unwrap();

    let state = harness.admin_state().unwrap();
    assert_eq!(
        state.queued().len(),
        1,
        "Multisig config update should be queued"
    );
    assert_eq!(
        state.next_update_id(),
        1,
        "Update ID should increment after queuing"
    );

    // Verify config hasn't changed yet (update is queued, not applied)
    let current_auth = state
        .authority(Role::StrataAdministrator)
        .expect("Admin authority should exist");
    assert_eq!(
        current_auth.config().keys().len(),
        initial_member_count,
        "Member count should not change until update is activated"
    );
}

/// Verifies predicate (verifying key) updates are queued.
#[tokio::test(flavor = "multi_thread")]
async fn test_predicate_update_is_queued() {
    let harness = create_test_harness().await.unwrap();
    let mut ctx = harness.admin_context();

    // Initialize subprotocols
    harness.mine_block(None).await.unwrap();

    let new_predicate = PredicateKey::always_accept();
    harness
        .submit_admin_action(&mut ctx, predicate_update(new_predicate, ProofType::OLStf))
        .await
        .unwrap();

    let state = harness.admin_state().unwrap();
    assert_eq!(state.queued().len(), 1, "Predicate update should be queued");
    assert_eq!(
        state.next_update_id(),
        1,
        "Update ID should increment after queuing"
    );
}

// ============================================================================
// Queued Update Activation
// ============================================================================

/// Verifies queued updates activate after confirmation_depth blocks.
#[tokio::test(flavor = "multi_thread")]
async fn test_queued_update_activates() {
    // In test params, confirmation_depth=1, so updates activate after 1 block
    let harness = create_test_harness().await.unwrap();
    let mut ctx = harness.admin_context();

    // Initialize subprotocols
    harness.mine_block(None).await.unwrap();

    let initial_state = harness.admin_state().unwrap();
    let initial_auth = initial_state
        .authority(Role::StrataAdministrator)
        .expect("Admin authority should exist");
    let initial_member_count = initial_auth.config().keys().len();

    // Generate a new public key to add
    let secp = Secp256k1::new();
    let new_privkey = SecretKey::new(&mut OsRng);
    let new_pubkey = PublicKey::from_secret_key(&secp, &new_privkey);
    let new_member = CompressedPublicKey::from(new_pubkey);

    // Submit multisig config update (gets queued)
    harness
        .submit_admin_action(
            &mut ctx,
            multisig_config_update(Role::StrataAdministrator, vec![new_member], vec![], 1),
        )
        .await
        .unwrap();

    // Verify update is queued but not applied yet
    let state = harness.admin_state().unwrap();
    assert_eq!(state.queued().len(), 1, "Update should be queued");
    let current_auth = state
        .authority(Role::StrataAdministrator)
        .expect("Admin authority should exist");
    assert_eq!(
        current_auth.config().keys().len(),
        initial_member_count,
        "Member count should not change until activation"
    );

    // Mine another block to trigger activation (confirmation_depth=1)
    harness.mine_block(None).await.unwrap();

    // Verify update has been activated
    let final_state = harness.admin_state().unwrap();
    assert_eq!(
        final_state.queued().len(),
        0,
        "Queue should be empty after activation"
    );
    let final_auth = final_state
        .authority(Role::StrataAdministrator)
        .expect("Admin authority should exist");
    assert_eq!(
        final_auth.config().keys().len(),
        initial_member_count + 1,
        "Member count should increase after activation"
    );

    assert!(
        final_auth.config().keys().contains(&new_member),
        "New member should be in the multisig config"
    );
}

// ============================================================================
// Cancel Actions
// ============================================================================

/// Verifies cancel action removes a queued update.
#[tokio::test(flavor = "multi_thread")]
async fn test_cancel_removes_queued_update() {
    let harness = create_test_harness().await.unwrap();
    let mut ctx = harness.admin_context();

    // Create an operator set update that gets queued (ID=0)
    harness
        .submit_admin_action(&mut ctx, operator_set_update(vec![[6u8; 32]], vec![]))
        .await
        .unwrap();

    let state = harness.admin_state().unwrap();
    assert_eq!(state.queued().len(), 1, "Update should be queued");
    assert_eq!(*state.queued()[0].id(), 0, "Queued update should have ID 0");

    // Cancel the queued update
    harness
        .submit_admin_action(&mut ctx, cancel_update(0))
        .await
        .unwrap();

    let state = harness.admin_state().unwrap();
    assert_eq!(state.queued().len(), 0, "Queued update should be cancelled");
    assert_eq!(
        state.next_update_id(),
        1,
        "Update ID should still be 1 after cancel"
    );
}

// ============================================================================
// Signature Validation
// ============================================================================

/// Verifies transactions signed with wrong key are rejected.
#[tokio::test(flavor = "multi_thread")]
async fn test_wrong_key_rejected() {
    let harness = create_test_harness().await.unwrap();

    // Create a transaction signed with WRONG key (not the operator key)
    let secp = Secp256k1::new();
    let wrong_privkey = SecretKey::new(&mut OsRng);
    let wrong_pubkey = PublicKey::from_secret_key(&secp, &wrong_privkey);
    let compressed_pk = CompressedPublicKey::from(wrong_pubkey);

    let _wrong_config =
        ThresholdConfig::try_new(vec![compressed_pk], NonZero::new(1).unwrap()).unwrap();

    // Sign with wrong key
    let action = sequencer_update([2u8; 32]);
    let sighash = action.compute_sighash(0);
    let sig_set = create_signature_set(&[wrong_privkey], &[0u8], sighash);
    let signed = SignedPayload::new(action.clone(), sig_set);
    let payload = borsh::to_vec(&signed).unwrap();

    let tx = harness
        .build_envelope_tx(
            ADMIN_SUBPROTOCOL_ID,
            action.tx_type(),
            payload,
            AsmTestHarness::DEFAULT_FEE,
        )
        .await
        .unwrap();

    let target_height = harness.get_processed_height().unwrap() + 1;
    harness.submit_and_mine_tx(&tx).await.unwrap();
    harness
        .wait_for_height(target_height, Duration::from_secs(5))
        .await
        .unwrap();

    let state = harness.admin_state().unwrap();
    assert_eq!(
        state.queued().len(),
        0,
        "Invalid tx should not queue updates"
    );
    assert_eq!(
        state.next_update_id(),
        0,
        "Update ID should not change for rejected tx"
    );
}

/// Verifies transactions with corrupted signatures are rejected.
#[tokio::test(flavor = "multi_thread")]
async fn test_corrupted_signature_rejected() {
    let harness = create_test_harness().await.unwrap();
    let ctx = harness.admin_context();

    let action = sequencer_update([88u8; 32]);
    let sighash = action.compute_sighash(0);
    let sig_set = create_signature_set(ctx.privkeys(), ctx.signer_indices(), sighash);

    // Corrupt the signature
    let mut indexed_sigs = sig_set.into_inner();
    if let Some(sig) = indexed_sigs.get_mut(0) {
        let index = sig.index();
        let mut sig_bytes = [0u8; 65];
        sig_bytes[0] = sig.recovery_id();
        sig_bytes[1..33].copy_from_slice(sig.r());
        sig_bytes[33..65].copy_from_slice(sig.s());
        sig_bytes[1] ^= 0xFF; // Corrupt r component
        *sig = IndexedSignature::new(index, sig_bytes);
    }

    let corrupted_sig_set = SignatureSet::new(indexed_sigs).unwrap();
    let signed = SignedPayload::new(action.clone(), corrupted_sig_set);
    let payload = borsh::to_vec(&signed).unwrap();

    let tx = harness
        .build_envelope_tx(
            ADMIN_SUBPROTOCOL_ID,
            action.tx_type(),
            payload,
            AsmTestHarness::DEFAULT_FEE,
        )
        .await
        .unwrap();

    let target_height = harness.get_processed_height().unwrap() + 1;
    harness.submit_and_mine_tx(&tx).await.unwrap();
    harness
        .wait_for_height(target_height, Duration::from_secs(5))
        .await
        .unwrap();

    let state = harness.admin_state().unwrap();
    assert_eq!(
        state.next_update_id(),
        0,
        "Corrupted signature should be rejected"
    );
}

// ============================================================================
// Replay Protection
// ============================================================================

/// Verifies replay attacks (reused sequence numbers) are rejected.
#[tokio::test(flavor = "multi_thread")]
async fn test_replay_attack_rejected() {
    let harness = create_test_harness().await.unwrap();
    let mut ctx = harness.admin_context();

    // Submit first transaction (seqno=0, auto-incremented to 1)
    harness
        .submit_admin_action(&mut ctx, sequencer_update([4u8; 32]))
        .await
        .unwrap();

    // Try to replay with seqno=0 (should fail)
    harness
        .submit_admin_action_with_seqno(&ctx, sequencer_update([5u8; 32]), 0)
        .await
        .unwrap();

    let state = harness.admin_state().unwrap();

    assert_eq!(
        state.next_update_id(),
        1,
        "Only first tx should be processed (replay rejected)"
    );
    assert_eq!(state.queued().len(), 0, "No updates should be queued");
}

// ============================================================================
// Multiple Operations
// ============================================================================

/// Verifies multiple admin transactions can be processed in a single block.
#[tokio::test(flavor = "multi_thread")]
async fn test_multiple_updates_same_block() {
    let harness = create_test_harness().await.unwrap();
    let mut ctx = harness.admin_context();

    // Build 3 transactions with sequential seqnos
    let (payload1, tx_type1) = ctx.sign(sequencer_update([7u8; 32]));
    let (payload2, tx_type2) = ctx.sign(sequencer_update([8u8; 32]));
    let (payload3, tx_type3) = ctx.sign(sequencer_update([9u8; 32]));

    let tx1 = harness
        .build_envelope_tx(
            ADMIN_SUBPROTOCOL_ID,
            tx_type1,
            payload1,
            AsmTestHarness::DEFAULT_FEE,
        )
        .await
        .unwrap();
    let tx2 = harness
        .build_envelope_tx(
            ADMIN_SUBPROTOCOL_ID,
            tx_type2,
            payload2,
            AsmTestHarness::DEFAULT_FEE,
        )
        .await
        .unwrap();
    let tx3 = harness
        .build_envelope_tx(
            ADMIN_SUBPROTOCOL_ID,
            tx_type3,
            payload3,
            AsmTestHarness::DEFAULT_FEE,
        )
        .await
        .unwrap();

    // Submit all 3 to mempool
    harness.submit_transaction(&tx1).await.unwrap();
    harness.submit_transaction(&tx2).await.unwrap();
    harness.submit_transaction(&tx3).await.unwrap();

    let target_height = harness.get_processed_height().unwrap() + 1;

    // Mine single block containing all
    let block_hash = harness.mine_block(None).await.unwrap();
    harness
        .wait_for_height(target_height, Duration::from_secs(5))
        .await
        .unwrap();

    // Verify all 3 transactions were included in the block
    let block = harness.client.get_block(&block_hash).await.unwrap();
    let parser = ParseConfig::new(harness.params.rollup().magic_bytes);
    let admin_tx_count = block
        .txdata
        .iter()
        .filter(|tx| {
            parser
                .try_parse_tx(tx)
                .map(|payload| payload.subproto_id() == ADMIN_SUBPROTOCOL_ID)
                .unwrap_or(false)
        })
        .count();

    assert_eq!(
        admin_tx_count, 3,
        "Expected all 3 admin transactions in block"
    );

    let state = harness.admin_state().unwrap();
    assert_eq!(
        state.queued().len(),
        0,
        "Sequencer updates apply immediately"
    );

    // Note: Due to mempool reordering, not all may process successfully
    let processed = state.next_update_id();
    assert!(
        (1..=3).contains(&processed),
        "Expected 1-3 transactions to process, got {}",
        processed
    );
}
