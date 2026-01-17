use std::cmp::Ordering;

use ssz::Encode;
use ssz_primitives::FixedBytes;
use strata_asm_common::VerifiedAuxData;
use strata_checkpoint_types_ssz::{
    CheckpointClaim, CheckpointPayload, CheckpointTip, L2BlockRange, SignedCheckpointPayload,
};
use strata_crypto::hash;
use strata_identifiers::Epoch;

use crate::{
    errors::{CheckpointError, CheckpointResult},
    state::CheckpointState,
};

/// Validates a checkpoint payload.
///
/// Performs three critical validation steps:
/// 1. Verifies the sequencer signature using the sequencer predicate
/// 2. Validates that the checkpoint advances to the next expected epoch
/// 3. Constructs the full checkpoint claim and verifies its proof
pub fn validate_checkpoint_payload(
    state: &CheckpointState,
    current_l1_height: u32,
    payload: &SignedCheckpointPayload,
    verified_aux_data: &VerifiedAuxData,
) -> CheckpointResult<()> {
    // Verify sequencer signature over payload
    // BIP-340 Schnorr verification hashes the message internally using tagged hashing,
    // so we pass raw SSZ-encoded bytes (not pre-hashed)
    let payload_bytes = payload.inner.as_ssz_bytes();
    state
        .sequencer_predicate()
        .verify_claim_witness(&payload_bytes, payload.signature.as_ref())
        .map_err(|_| CheckpointError::InvalidSignature)?;

    // Validate epoch progression
    let expected_epoch = state.verified_tip().epoch + 1;
    if payload.inner().new_tip().epoch != expected_epoch {
        return Err(CheckpointError::InvalidEpoch {
            expected: expected_epoch,
            actual: payload.inner().new_tip().epoch,
        });
    }

    // Construct full checkpoint claim and verify its proof
    let claim = construct_full_claim(
        expected_epoch,
        current_l1_height,
        &state.verified_tip,
        payload.inner(),
        verified_aux_data,
    )?;
    state
        .checkpoint_predicate()
        .verify_claim_witness(&claim.as_ssz_bytes(), payload.inner.proof())?;

    Ok(())
}

/// Constructs a complete checkpoint claim for verification.
fn construct_full_claim(
    epoch: Epoch,
    current_l1_height: u32,
    verified_tip: &CheckpointTip,
    payload: &CheckpointPayload,
    verified_aux_data: &VerifiedAuxData,
) -> CheckpointResult<CheckpointClaim> {
    let l2_range = L2BlockRange::new(
        *verified_tip.l2_commitment(),
        payload.new_tip().l2_commitment,
    );

    let l1_height_covered_in_last_checkpoint = verified_tip.l1_height();
    let l1_height_covered_in_new_checkpoint = payload.new_tip().l1_height();

    if l1_height_covered_in_new_checkpoint >= current_l1_height {
        return Err(CheckpointError::CheckpointBeyondL1Tip {
            checkpoint_height: l1_height_covered_in_new_checkpoint,
            current_height: current_l1_height,
        });
    }

    // Compute ASM manifests hash based on L1 height progression
    let asm_manifests_hash =
        match l1_height_covered_in_last_checkpoint.cmp(&l1_height_covered_in_new_checkpoint) {
            // Invalid: checkpoint goes backwards in L1 height
            Ordering::Greater => {
                return Err(CheckpointError::L1HeightGoesBackwards {
                    prev_height: l1_height_covered_in_last_checkpoint,
                    new_height: l1_height_covered_in_new_checkpoint,
                });
            }
            // Valid: checkpoint advances L2 state without consuming new L1 blocks
            Ordering::Equal => FixedBytes::<32>::from([0u8; 32]),

            // Valid: checkpoint processes new L1 blocks
            // Start from (prev_checkpoint_l1_height + 1) since prev_checkpoint_l1_height
            // was already processed in the previous checkpoint
            Ordering::Less => {
                let manifest_hashes = verified_aux_data.get_manifest_hashes(
                    (l1_height_covered_in_last_checkpoint + 1) as u64,
                    l1_height_covered_in_last_checkpoint as u64,
                )?;

                compute_asm_manifests_hash(manifest_hashes)
            }
        };

    let state_diff_hash = hash::raw(payload.sidecar().ol_state_diff()).into();

    // Hash SSZ-encoded OL logs (convert to Vec for SSZ encoding)
    let ol_logs_vec = payload.sidecar().ol_logs().to_vec();
    let ol_logs_hash = hash::raw(&ol_logs_vec.as_ssz_bytes()).into();

    Ok(CheckpointClaim::new(
        epoch,
        l2_range,
        asm_manifests_hash,
        state_diff_hash,
        ol_logs_hash,
    ))
}

/// Computes a hash commitment over all ASM manifests in an L1 block range.
///
/// Concatenates the manifest hashes for all L1 blocks in the range
/// and returns a single hash commitment over them.
fn compute_asm_manifests_hash(manifest_hashes: Vec<[u8; 32]>) -> FixedBytes<32> {
    let mut data = Vec::with_capacity(manifest_hashes.len() * 32);
    for h in manifest_hashes {
        data.extend_from_slice(h.as_ref());
    }
    let hash = hash::raw(&data);
    hash.into()
}
