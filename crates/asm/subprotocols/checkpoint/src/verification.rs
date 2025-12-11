//! Checkpoint verification logic.
//!
//! This module handles signature verification, claim construction,
//! and proof verification for checkpoints.

use strata_asm_manifest_types::Hash32;
use strata_checkpoint_types_new::{
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
