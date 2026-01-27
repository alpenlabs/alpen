use bitcoin_bosd::Descriptor;
use ssz::Encode;
use ssz_primitives::FixedBytes;
use strata_asm_bridge_msgs::WithdrawOutput;
use strata_asm_common::{VerifiedAuxData, logging};
use strata_checkpoint_types_ssz::{
    CheckpointClaim, CheckpointSidecar, CheckpointTip, L2BlockRange, OLLog,
    SignedCheckpointPayload, compute_asm_manifests_hash,
};
use strata_codec::decode_buf_exact;
use strata_crypto::hash;
use strata_ol_chain_types_new::SimpleWithdrawalIntentLogData;
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;

use crate::{
    errors::{CheckpointValidationResult, InvalidCheckpointPayload},
    state::CheckpointState,
};

/// Validates a checkpoint payload and extracts withdrawal intents.
///
/// This is the single validation function for checkpoint payloads. Once validation succeeds,
/// the payload can be safely acted upon. Returns extracted withdrawal intents on success.
pub fn validate_checkpoint_and_extract_withdrawal_intents(
    state: &CheckpointState,
    current_l1_height: u32,
    payload: &SignedCheckpointPayload,
    verified_aux_data: &VerifiedAuxData,
) -> CheckpointValidationResult<Vec<WithdrawOutput>> {
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

    // 3. Validate L1 progression
    let l1_height_covered_in_last_checkpoint = state.verified_tip.l1_height();
    let l1_height_covered_in_new_checkpoint = payload.inner().new_tip.l1_height();

    // 3a. Invalid: checkpoint exceeds the current L1 tip
    if l1_height_covered_in_new_checkpoint >= current_l1_height {
        return Err(InvalidCheckpointPayload::CheckpointBeyondL1Tip {
            checkpoint_height: l1_height_covered_in_new_checkpoint,
            current_height: current_l1_height,
        }
        .into());
    }

    // 3b. Invalid: checkpoint goes backwards in L1 height
    if l1_height_covered_in_last_checkpoint > l1_height_covered_in_new_checkpoint {
        return Err(InvalidCheckpointPayload::L1HeightGoesBackwards {
            prev_height: l1_height_covered_in_last_checkpoint,
            new_height: l1_height_covered_in_new_checkpoint,
        }
        .into());
    }

    // 4. Validate L2 progression
    let prev_slot = state.verified_tip().l2_commitment().slot();
    let new_slot = payload.inner().new_tip().l2_commitment().slot();
    if new_slot <= prev_slot {
        return Err(InvalidCheckpointPayload::L2SlotDoesNotAdvance {
            prev_slot,
            new_slot,
        }
        .into());
    }

    // 5. Construct full checkpoint claim
    let asm_manifests_hash = compute_asm_manifests_hash_for_checkpoint(
        l1_height_covered_in_last_checkpoint + 1,
        l1_height_covered_in_new_checkpoint,
        verified_aux_data,
    )?;
    let claim = construct_full_claim(
        &state.verified_tip,
        payload.inner().new_tip(),
        payload.inner().sidecar(),
        asm_manifests_hash,
    )?;

    // 6. Verify the proof
    state
        .checkpoint_predicate()
        .verify_claim_witness(&claim.as_ssz_bytes(), payload.inner.proof())
        .map_err(InvalidCheckpointPayload::CheckpointPredicateVerification)?;

    // 7. Extract and validate withdrawal intent logs
    let withdrawal_intents =
        extract_and_validate_withdrawal_intents(payload.inner().sidecar().ol_logs())?;

    Ok(withdrawal_intents)
}

/// Constructs a complete checkpoint claim for verification by combining the verified tip state
/// with the new checkpoint payload.
fn construct_full_claim(
    verified_tip: &CheckpointTip,
    new_tip: &CheckpointTip,
    sidecar: &CheckpointSidecar,
    asm_manifests_hash: FixedBytes<32>,
) -> CheckpointValidationResult<CheckpointClaim> {
    let l2_range = L2BlockRange::new(*verified_tip.l2_commitment(), new_tip.l2_commitment);

    let state_diff_hash = hash::raw(sidecar.ol_state_diff()).into();

    // Hash SSZ-encoded OL logs (convert to Vec for SSZ encoding)
    let ol_logs_vec = sidecar.ol_logs().to_vec();
    let ol_logs_hash = hash::raw(&ol_logs_vec.as_ssz_bytes()).into();

    Ok(CheckpointClaim::new(
        new_tip.epoch,
        l2_range,
        asm_manifests_hash,
        state_diff_hash,
        ol_logs_hash,
    ))
}

/// Computes the ASM manifests hash for a range of L1 blocks.
///
/// Returns an error if the manifest hashes cannot be retrieved from aux data.
fn compute_asm_manifests_hash_for_checkpoint(
    start_height: u32,
    end_height: u32,
    verified_aux_data: &VerifiedAuxData,
) -> CheckpointValidationResult<FixedBytes<32>> {
    let manifest_hashes =
        verified_aux_data.get_manifest_hashes(start_height as u64, end_height as u64)?;

    Ok(compute_asm_manifests_hash(&manifest_hashes))
}

/// Extracts and validates withdrawal intent logs from OL logs.
///
/// Filters OL logs from the bridge gateway account, validates that withdrawal intent
/// destination descriptors can be parsed, and returns the extracted withdrawal outputs.
fn extract_and_validate_withdrawal_intents(
    logs: &[OLLog],
) -> CheckpointValidationResult<Vec<WithdrawOutput>> {
    let mut withdrawal_intents = Vec::new();

    for log in logs
        .iter()
        .filter(|l| l.account_serial() == BRIDGE_GATEWAY_ACCT_SERIAL)
    {
        // Attempt to decode as withdrawal intent log data
        // Logs from this account may have other formats, so skip if decoding fails
        let Ok(withdrawal_data) = decode_buf_exact::<SimpleWithdrawalIntentLogData>(log.payload())
        else {
            logging::debug!("Skipping log that is not a withdrawal intent");
            continue;
        };

        // Parse destination descriptor; return error on malformed descriptors
        let Ok(destination) = Descriptor::from_bytes(withdrawal_data.dest()) else {
            // CRITICAL: User funds are destroyed on L2 but cannot be withdrawn on L1.
            // Since the extraction is done after the proof verification, this should have been a
            // proper descriptor.
            logging::error!("Failed to parse withdrawal destination descriptor");
            return Err(InvalidCheckpointPayload::MalformedWithdrawalDestDesc.into());
        };

        let withdraw_output = WithdrawOutput::new(destination, withdrawal_data.amt().into());
        withdrawal_intents.push(withdraw_output);
    }

    Ok(withdrawal_intents)
}

#[cfg(test)]
mod tests {
    use strata_asm_common::{AsmHistoryAccumulatorState, AuxData, VerifiedAuxData};
    use strata_identifiers::{AccountSerial, Buf64};
    use strata_ol_chain_types_new::OLLog;
    use strata_test_utils_l2::CheckpointTestHarness;

    use crate::{
        errors::{CheckpointValidationError, InvalidCheckpointPayload},
        state::CheckpointState,
        verification::{
            compute_asm_manifests_hash_for_checkpoint,
            validate_checkpoint_and_extract_withdrawal_intents,
        },
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

        let res = validate_checkpoint_and_extract_withdrawal_intents(
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

        let err = validate_checkpoint_and_extract_withdrawal_intents(
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
    fn test_invalid_epoch_progression() {
        let (state, harness) = test_setup();
        let mut payload = harness.build_payload();
        payload.new_tip.epoch = state.verified_tip().epoch + 2;
        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());
        let signed_payload = harness.sign_payload(payload);

        let current_l1_height = signed_payload.inner().new_tip().l1_height + 1;

        let err = validate_checkpoint_and_extract_withdrawal_intents(
            &state,
            current_l1_height,
            &signed_payload,
            &verified_aux_data,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            CheckpointValidationError::InvalidPayload(
                InvalidCheckpointPayload::InvalidEpoch { .. }
            )
        ));
    }

    #[test]
    fn test_new_tip_beyond_current_l1_height() {
        let (state, harness) = test_setup();
        let payload = harness.build_payload();
        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());
        let signed_payload = harness.sign_payload(payload);

        let current_l1_height = signed_payload.inner().new_tip().l1_height - 1;

        let err = validate_checkpoint_and_extract_withdrawal_intents(
            &state,
            current_l1_height,
            &signed_payload,
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
    fn test_new_l1_tip_goes_backwards() {
        let (state, harness) = test_setup();
        let mut payload = harness.build_payload();
        payload.new_tip.l1_height = state.verified_tip().l1_height - 1;
        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());
        let signed_payload = harness.sign_payload(payload);

        let current_l1_height = state.verified_tip().l1_height + 1;

        let err = validate_checkpoint_and_extract_withdrawal_intents(
            &state,
            current_l1_height,
            &signed_payload,
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
    fn test_new_l1_tip_same_as_last_verified() {
        let (state, harness) = test_setup();

        let mut new_tip = harness.gen_new_tip();
        new_tip.l1_height = state.verified_tip().l1_height;
        let payload = harness.build_payload_with_tip(new_tip);

        let verified_aux_data = harness.gen_verified_aux(&new_tip);
        let signed_payload = harness.sign_payload(payload);

        let current_l1_height = state.verified_tip().l1_height + 1;

        let res = validate_checkpoint_and_extract_withdrawal_intents(
            &state,
            current_l1_height,
            &signed_payload,
            &verified_aux_data,
        );
        assert!(res.is_ok());
    }

    #[test]
    fn test_l2_slot_does_not_advance() {
        let (state, harness) = test_setup();
        let mut payload = harness.build_payload();
        let verified_aux_data = harness.gen_verified_aux(payload.new_tip());

        // Set new L2 slot to be equal to the previous slot (no progression)
        payload.new_tip.l2_commitment = *state.verified_tip().l2_commitment();

        let current_l1_height = payload.new_tip().l1_height + 1;
        let signed_payload = harness.sign_payload(payload);

        let err = validate_checkpoint_and_extract_withdrawal_intents(
            &state,
            current_l1_height,
            &signed_payload,
            &verified_aux_data,
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CheckpointValidationError::InvalidPayload(
                InvalidCheckpointPayload::L2SlotDoesNotAdvance { .. }
            )
        ));
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

        let err = compute_asm_manifests_hash_for_checkpoint(
            state.verified_tip.l1_height() + 1,
            payload.new_tip().l1_height(),
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

        let err = validate_checkpoint_and_extract_withdrawal_intents(
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

        let err = validate_checkpoint_and_extract_withdrawal_intents(
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

        let err = validate_checkpoint_and_extract_withdrawal_intents(
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
