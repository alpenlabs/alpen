use std::cmp::Ordering;

use ssz::Encode;
use ssz_primitives::FixedBytes;
use strata_asm_common::VerifiedAuxData;
use strata_checkpoint_types_ssz::{
    CheckpointClaim, CheckpointPayload, CheckpointTip, L2BlockRange, SignedCheckpointPayload,
    compute_asm_manifests_hash,
};
use strata_crypto::hash;
use strata_identifiers::Epoch;

use crate::{
    errors::{CheckpointValidationResult, InvalidCheckpointPayload},
    state::CheckpointState,
};

/// Validates a checkpoint payload by verifying the sequencer signature, epoch progression,
/// and checkpoint proof.
///
/// The full [`CheckpointClaim`] is reconstructed from the current subprotocol state and payload
/// for the full proof verification.
pub fn validate_checkpoint_payload(
    state: &CheckpointState,
    current_l1_height: u32,
    payload: &SignedCheckpointPayload,
    verified_aux_data: &VerifiedAuxData,
) -> CheckpointValidationResult<()> {
    // 1. Verify sequencer signature over payload
    // BIP-340 Schnorr verification hashes the message internally using tagged hashing,
    // so we pass raw SSZ-encoded bytes (not pre-hashed)
    let payload_bytes = payload.inner.as_ssz_bytes();
    state
        .sequencer_predicate()
        .verify_claim_witness(&payload_bytes, payload.signature.as_ref())
        .map_err(InvalidCheckpointPayload::SequencerPredicateVerification)?;

    // 2. Validate epoch progression
    let expected_epoch = state.verified_tip().epoch + 1;
    if payload.inner().new_tip().epoch != expected_epoch {
        return Err(InvalidCheckpointPayload::InvalidEpoch {
            expected: expected_epoch,
            actual: payload.inner().new_tip().epoch,
        }
        .into());
    }

    // 3a.Construct full checkpoint claim and verify its proof
    let claim = construct_full_claim(
        expected_epoch,
        current_l1_height,
        &state.verified_tip,
        payload.inner(),
        verified_aux_data,
    )?;

    // 3b. Verify the proof
    state
        .checkpoint_predicate()
        .verify_claim_witness(&claim.as_ssz_bytes(), payload.inner.proof())
        .map_err(InvalidCheckpointPayload::CheckpointPredicateVerification)?;

    Ok(())
}

/// Constructs a complete checkpoint claim for verification by combining the verified tip state
/// with the new checkpoint payload.
fn construct_full_claim(
    epoch: Epoch,
    current_l1_height: u32,
    verified_tip: &CheckpointTip,
    payload: &CheckpointPayload,
    verified_aux_data: &VerifiedAuxData,
) -> CheckpointValidationResult<CheckpointClaim> {
    let l2_range = L2BlockRange::new(
        *verified_tip.l2_commitment(),
        payload.new_tip().l2_commitment,
    );

    let asm_manifests_hash = compute_asm_manifests_hash_for_checkpoint(
        verified_tip,
        payload.new_tip(),
        current_l1_height,
        verified_aux_data,
    )?;

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

/// Computes the ASM manifests hash for a checkpoint transition.
///
/// Validates L1 height progression between the previous and new checkpoint tips:
/// - Returns an error if the new checkpoint goes backwards in L1 height
/// - Returns an error if the new checkpoint exceeds the current L1 tip
/// - Returns a zero hash if no new L1 blocks were processed (L1 height unchanged)
/// - Otherwise computes and returns the hash of all ASM manifests from the L1 block range
fn compute_asm_manifests_hash_for_checkpoint(
    verified_tip: &CheckpointTip,
    new_tip: &CheckpointTip,
    current_l1_height: u32,
    verified_aux_data: &VerifiedAuxData,
) -> CheckpointValidationResult<FixedBytes<32>> {
    let l1_height_covered_in_last_checkpoint = verified_tip.l1_height();
    let l1_height_covered_in_new_checkpoint = new_tip.l1_height();

    if l1_height_covered_in_new_checkpoint >= current_l1_height {
        return Err(InvalidCheckpointPayload::CheckpointBeyondL1Tip {
            checkpoint_height: l1_height_covered_in_new_checkpoint,
            current_height: current_l1_height,
        }
        .into());
    }

    match l1_height_covered_in_last_checkpoint.cmp(&l1_height_covered_in_new_checkpoint) {
        // Invalid: checkpoint goes backwards in L1 height
        Ordering::Greater => Err(InvalidCheckpointPayload::L1HeightGoesBackwards {
            prev_height: l1_height_covered_in_last_checkpoint,
            new_height: l1_height_covered_in_new_checkpoint,
        }
        .into()),

        // Valid: checkpoint advances L2 state without consuming new L1 blocks
        Ordering::Equal => Ok(FixedBytes::<32>::from([0u8; 32])),

        // Valid: checkpoint processes new L1 blocks
        // Start from (prev_checkpoint_l1_height + 1) since prev_checkpoint_l1_height
        // was already processed in the previous checkpoint
        Ordering::Less => {
            let manifest_hashes = verified_aux_data.get_manifest_hashes(
                (l1_height_covered_in_last_checkpoint + 1) as u64,
                l1_height_covered_in_new_checkpoint as u64,
            )?;

            Ok(compute_asm_manifests_hash(manifest_hashes))
        }
    }
}

#[cfg(test)]
mod tests {
    use ssz_primitives::FixedBytes;
    use strata_asm_common::{AsmHistoryAccumulatorState, AuxData, VerifiedAuxData};
    use strata_identifiers::{AccountSerial, Buf64};
    use strata_ol_chain_types_new::OLLog;
    use strata_test_utils_l2::CheckpointTestHarness;

    use crate::{
        errors::{CheckpointValidationError, InvalidCheckpointPayload},
        state::CheckpointState,
        verification::{compute_asm_manifests_hash_for_checkpoint, validate_checkpoint_payload},
    };

    fn test_setup() -> (CheckpointState, CheckpointTestHarness) {
        let harness = CheckpointTestHarness::new_random();
        let state = CheckpointState::new(
            harness.sequencer_predicate(),
            harness.checkpoint_predicate(),
            *harness.verified_tip(),
        );
        (state, harness)
    }

    #[test]
    fn test_validate_checkpoint_success() {
        let (state, harness) = test_setup();
        let payload = harness.build_payload();
        let new_tip = *payload.new_tip();

        let signed_payload = harness.sign_payload(payload);
        let verified_aux_data = &harness.gen_verified_aux(&new_tip);

        let current_l1_height = new_tip.l1_height + 1;

        let res = validate_checkpoint_payload(
            &state,
            current_l1_height,
            &signed_payload,
            verified_aux_data,
        );
        assert!(res.is_ok());
    }

    #[test]
    fn test_invalid_signature() {
        let (state, harness) = test_setup();
        let payload = harness.build_payload();
        let current_l1_height = payload.new_tip().l1_height;
        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());
        let mut signed_payload = harness.sign_payload(payload);

        signed_payload.signature = Buf64::zero();

        let err = validate_checkpoint_payload(
            &state,
            current_l1_height,
            &signed_payload,
            &verified_aux_data,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CheckpointValidationError::InvalidPayload(
                InvalidCheckpointPayload::SequencerPredicateVerification(_)
            )
        ));
    }

    #[test]
    fn test_asm_manifests_hash_computation_payload_beyond_l1_tip() {
        let (state, harness) = test_setup();
        let payload = harness.build_payload();
        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());

        let current_l1_height = payload.new_tip().l1_height - 1;

        let err = compute_asm_manifests_hash_for_checkpoint(
            &state.verified_tip,
            payload.new_tip(),
            current_l1_height,
            &verified_aux_data,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CheckpointValidationError::InvalidPayload(
                InvalidCheckpointPayload::CheckpointBeyondL1Tip { .. }
            )
        ))
    }

    #[test]
    fn test_asm_manifests_hash_computation_l1_no_progress() {
        let (state, harness) = test_setup();
        let mut payload = harness.build_payload();
        let verified_aux_data = harness.gen_verified_aux(&payload.new_tip);

        payload.new_tip.l1_height = state.verified_tip().l1_height;

        let current_l1_height = state.verified_tip().l1_height + 1;

        let res = compute_asm_manifests_hash_for_checkpoint(
            &state.verified_tip,
            payload.new_tip(),
            current_l1_height,
            &verified_aux_data,
        )
        .unwrap();
        assert_eq!(res, FixedBytes::zero())
    }

    #[test]
    fn test_asm_manifests_hash_computation_l1_backwards() {
        let (state, harness) = test_setup();
        let mut payload = harness.build_payload();
        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());
        payload.new_tip.l1_height = state.verified_tip().l1_height - 1;

        let current_l1_height = state.verified_tip().l1_height + 1;

        let err = compute_asm_manifests_hash_for_checkpoint(
            &state.verified_tip,
            payload.new_tip(),
            current_l1_height,
            &verified_aux_data,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CheckpointValidationError::InvalidPayload(
                InvalidCheckpointPayload::L1HeightGoesBackwards { .. }
            )
        ))
    }

    #[test]
    fn test_asm_manifests_hash_computation_invalid_aux() {
        let (state, harness) = test_setup();
        let payload = harness.build_payload();

        let aux_data = AuxData::new(vec![], vec![]);
        let asm_accumulator_state =
            AsmHistoryAccumulatorState::new(harness.genesis_l1_height() as u64);
        let verified_aux_data =
            VerifiedAuxData::try_new(&aux_data, &asm_accumulator_state).unwrap();

        let current_l1_height = payload.new_tip().l1_height + 1;

        let err = compute_asm_manifests_hash_for_checkpoint(
            &state.verified_tip,
            payload.new_tip(),
            current_l1_height,
            &verified_aux_data,
        )
        .unwrap_err();
        assert!(matches!(err, CheckpointValidationError::InvalidAux(_)));
    }

    #[test]
    fn test_invalid_state_diff() {
        let (state, harness) = test_setup();
        let mut payload = harness.build_payload();
        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());
        let current_l1_height = payload.new_tip().l1_height + 1;

        // Modify the payload to include invalid state diff after proof generation.
        payload.sidecar.ol_state_diff = vec![99u8; 88].into();
        let signed_payload = harness.sign_payload(payload);

        let err = validate_checkpoint_payload(
            &state,
            current_l1_height,
            &signed_payload,
            &verified_aux_data,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CheckpointValidationError::InvalidPayload(
                InvalidCheckpointPayload::CheckpointPredicateVerification(_)
            )
        ));
    }

    #[test]
    fn test_invalid_ol_logs() {
        let (state, harness) = test_setup();
        let mut payload = harness.build_payload();
        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());
        let current_l1_height = payload.new_tip().l1_height + 1;

        // Modify the payload to include OL Logs that wasn't covered by the proof.
        let dummy_log = OLLog::new(AccountSerial::zero(), Vec::new());
        payload.sidecar.ol_logs = vec![dummy_log].into();

        let signed_payload = harness.sign_payload(payload);

        let err = validate_checkpoint_payload(
            &state,
            current_l1_height,
            &signed_payload,
            &verified_aux_data,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CheckpointValidationError::InvalidPayload(
                InvalidCheckpointPayload::CheckpointPredicateVerification(_)
            )
        ));
    }

    #[test]
    fn test_invalid_ol_l1_progression() {
        let (state, harness) = test_setup();
        let mut payload = harness.build_payload();

        let current_l1_height = payload.new_tip().l1_height + 100;

        // Modify the payload to include more L1 blocks after proof generation.
        payload.new_tip.l1_height += 10;

        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());

        let signed_payload = harness.sign_payload(payload);

        let err = validate_checkpoint_payload(
            &state,
            current_l1_height,
            &signed_payload,
            &verified_aux_data,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CheckpointValidationError::InvalidPayload(
                InvalidCheckpointPayload::CheckpointPredicateVerification(_)
            )
        ));
    }
}
