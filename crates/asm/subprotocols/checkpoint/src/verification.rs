//! Checkpoint verification logic.
//!
//! This module handles signature verification, claim construction,
//! and proof verification for checkpoints.

use strata_asm_manifest_types::Hash32;
use strata_checkpoint_types_ssz::{
    CheckpointClaim, CheckpointClaimBuilder, CheckpointPayload, SignedCheckpointPayload,
    verify_checkpoint_payload_signature as verify_sig,
};
use strata_identifiers::{Buf32, CredRule, Epoch, L2BlockCommitment, Slot, hash::raw};
use strata_predicate::PredicateKey;

use crate::{error::CheckpointResult, state::CheckpointState};

/// Verify the signature on a signed checkpoint payload.
pub(crate) fn verify_checkpoint_signature(
    signed_checkpoint: &SignedCheckpointPayload,
    sequencer_cred: &CredRule,
) -> bool {
    verify_sig(signed_checkpoint, sequencer_cred)
}

/// Construct checkpoint claim for proof verification.
///
/// Builds the [`CheckpointClaim`] from:
/// - Start values from checkpoint state (pre_state_root, l1_start, l2_start)
/// - End values from checkpoint payload (post_state_root, l1_end, l2_end)
/// - Manifest hashes for input message commitment (from auxiliary data)
///
/// This function is purely constructive - it does not perform validation.
/// Validation of state transitions should be done separately before calling this.
pub(crate) fn construct_checkpoint_claim(
    state: &CheckpointState,
    payload: &CheckpointPayload,
    manifest_hashes: &[Hash32],
) -> CheckpointResult<CheckpointClaim> {
    // Get start values from state
    let pre_state_root = state
        .verified_epoch_summary
        .as_ref()
        .map(|s| *s.final_state())
        .unwrap_or_else(Buf32::zero);

    let l1_start = state.last_checkpoint_l1;

    let l2_start = state
        .last_l2_terminal()
        .copied()
        .unwrap_or_else(L2BlockCommitment::null);

    // Compute input_msgs_commitment as rolling hash of manifest hashes
    let input_msgs_commitment = compute_manifest_hashes_commitment(manifest_hashes);

    // Build claim using the builder with start values from state
    let claim = CheckpointClaimBuilder::new(payload, pre_state_root, l1_start, l2_start)
        .with_input_msgs_commitment(input_msgs_commitment)
        .build();

    Ok(claim)
}

/// Compute a commitment over manifest hashes.
///
/// This creates a rolling hash of all manifest hashes to commit to the
/// input messages from L1 in the specified block range.
fn compute_manifest_hashes_commitment(manifest_hashes: &[Hash32]) -> Buf32 {
    if manifest_hashes.is_empty() {
        return Buf32::zero();
    }

    // Concatenate all hashes and compute a single hash
    let mut data = Vec::with_capacity(manifest_hashes.len() * 32);
    for hash in manifest_hashes {
        data.extend_from_slice(hash.as_ref());
    }

    raw(&data)
}

/// Verify the checkpoint proof using the checkpoint predicate.
pub(crate) fn verify_checkpoint_proof(
    checkpoint_predicate: &PredicateKey,
    claim: &CheckpointClaim,
    proof: &[u8],
) -> CheckpointResult<()> {
    // Serialize claim using Borsh
    let claim_bytes = claim.to_bytes();

    checkpoint_predicate
        .verify_claim_witness(&claim_bytes, proof)
        .map_err(|_| crate::error::CheckpointError::ProofVerification)
}

/// Validate that the checkpoint epoch follows sequentially from state.
pub(crate) fn validate_epoch_sequence(
    state: &CheckpointState,
    checkpoint_epoch: Epoch,
) -> CheckpointResult<()> {
    let expected = state.expected_next_epoch();
    if checkpoint_epoch != expected {
        return Err(crate::error::CheckpointError::InvalidEpoch {
            expected,
            actual: checkpoint_epoch,
        });
    }
    Ok(())
}

/// Validate L1 height progression.
pub(crate) fn validate_l1_progression(
    state: &CheckpointState,
    new_l1_height: u64,
) -> CheckpointResult<()> {
    let previous = state.last_checkpoint_l1.height_u64();
    if new_l1_height <= previous {
        return Err(crate::error::CheckpointError::InvalidL1Height {
            previous,
            new: new_l1_height,
        });
    }
    Ok(())
}

/// Validate L2 slot progression.
pub(crate) fn validate_l2_progression(
    state: &CheckpointState,
    new_l2_slot: Slot,
) -> CheckpointResult<()> {
    if let Some(terminal) = state.last_l2_terminal() {
        let previous = terminal.slot();
        if new_l2_slot <= previous {
            return Err(crate::error::CheckpointError::InvalidL2Slot {
                previous,
                new: new_l2_slot,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_predicate::PredicateKey;
    use strata_test_utils_asm::checkpoint::{
        CheckpointFixtures, SequencerKeypair, gen_checkpoint_payload, gen_l1_block_commitment,
    };

    use super::*;
    use crate::state::CheckpointConfig;

    fn create_test_config_with_fixtures(fixtures: &CheckpointFixtures) -> CheckpointConfig {
        CheckpointConfig {
            sequencer_cred: CredRule::SchnorrKey(fixtures.sequencer.public_key),
            checkpoint_predicate: PredicateKey::always_accept(),
            genesis_l1_block: gen_l1_block_commitment(100),
        }
    }

    #[test]
    fn test_verify_checkpoint_signature_valid() {
        let fixtures = CheckpointFixtures::new();
        let signed = fixtures.gen_signed_payload();

        let cred_rule = CredRule::SchnorrKey(fixtures.sequencer.public_key);
        assert!(verify_checkpoint_signature(&signed, &cred_rule));
    }

    #[test]
    fn test_verify_checkpoint_signature_invalid() {
        let fixtures = CheckpointFixtures::new();
        let signed = fixtures.gen_signed_payload();

        // Different keypair
        let wrong_keypair = SequencerKeypair::random();
        let wrong_cred = CredRule::SchnorrKey(wrong_keypair.public_key);

        assert!(!verify_checkpoint_signature(&signed, &wrong_cred));
    }

    #[test]
    fn test_verify_checkpoint_signature_unchecked() {
        let fixtures = CheckpointFixtures::new();
        let signed = fixtures.gen_signed_payload();

        // Unchecked always passes
        let unchecked_cred = CredRule::Unchecked;
        assert!(verify_checkpoint_signature(&signed, &unchecked_cred));
    }

    #[test]
    fn test_validate_epoch_sequence_valid() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        // Initial state expects epoch 0
        assert!(validate_epoch_sequence(&state, 0).is_ok());
    }

    #[test]
    fn test_validate_epoch_sequence_invalid() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        // Initial state expects epoch 0, not epoch 1
        let result = validate_epoch_sequence(&state, 1);
        assert!(result.is_err());

        if let Err(crate::error::CheckpointError::InvalidEpoch { expected, actual }) = result {
            assert_eq!(expected, 0);
            assert_eq!(actual, 1);
        } else {
            panic!("Expected InvalidEpoch error");
        }
    }

    #[test]
    fn test_validate_epoch_sequence_after_update() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let mut state = CheckpointState::new(&config);

        // Apply epoch 0
        let payload_0 = gen_checkpoint_payload(0);
        state.update_with_checkpoint(&payload_0);

        // Now epoch 1 is valid
        assert!(validate_epoch_sequence(&state, 1).is_ok());

        // But epoch 0 and epoch 2 are invalid
        assert!(validate_epoch_sequence(&state, 0).is_err());
        assert!(validate_epoch_sequence(&state, 2).is_err());
    }

    #[test]
    fn test_validate_l1_progression_valid() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        // Genesis at height 100, so any height > 100 is valid
        assert!(validate_l1_progression(&state, 101).is_ok());
        assert!(validate_l1_progression(&state, 200).is_ok());
    }

    #[test]
    fn test_validate_l1_progression_invalid() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        // Genesis at height 100, so height <= 100 is invalid
        let result = validate_l1_progression(&state, 100);
        assert!(result.is_err());

        let result = validate_l1_progression(&state, 50);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_l2_progression_initial() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        // No previous terminal, any slot is valid
        assert!(validate_l2_progression(&state, Slot::from(0u64)).is_ok());
        assert!(validate_l2_progression(&state, Slot::from(100u64)).is_ok());
    }

    #[test]
    fn test_validate_l2_progression_after_checkpoint() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let mut state = CheckpointState::new(&config);

        // Apply epoch 0
        let payload_0 = gen_checkpoint_payload(0);
        state.update_with_checkpoint(&payload_0);

        let terminal_slot = state.last_l2_terminal().unwrap().slot();

        // New slot must be > terminal_slot
        assert!(validate_l2_progression(&state, terminal_slot + 1).is_ok());
        assert!(validate_l2_progression(&state, terminal_slot + 100).is_ok());

        // Same or lower slot is invalid
        assert!(validate_l2_progression(&state, terminal_slot).is_err());
        if terminal_slot > 0 {
            assert!(validate_l2_progression(&state, terminal_slot - 1).is_err());
        }
    }

    #[test]
    fn test_construct_checkpoint_claim() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        let payload = gen_checkpoint_payload(0);
        let manifest_hashes: Vec<Hash32> = vec![];

        let claim = construct_checkpoint_claim(&state, &payload, &manifest_hashes).unwrap();

        // Verify claim fields
        assert_eq!(claim.epoch(), 0);
        assert_eq!(
            claim.post_state_root(),
            payload.transition().post_state_root()
        );
    }

    #[test]
    fn test_construct_checkpoint_claim_with_manifests() {
        let fixtures = CheckpointFixtures::new();
        let config = create_test_config_with_fixtures(&fixtures);
        let state = CheckpointState::new(&config);

        let payload = gen_checkpoint_payload(0);

        // Create some manifest hashes
        let manifest_hashes: Vec<Hash32> = vec![
            Hash32::from([1u8; 32]),
            Hash32::from([2u8; 32]),
            Hash32::from([3u8; 32]),
        ];

        let claim = construct_checkpoint_claim(&state, &payload, &manifest_hashes).unwrap();

        // Input msgs commitment should not be zero when we have manifests
        assert_ne!(*claim.input_msgs_commitment(), Buf32::zero());
    }
}
