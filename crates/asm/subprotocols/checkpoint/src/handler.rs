use ssz::Encode;
use ssz_primitives::FixedBytes;
use strata_asm_common::{MsgRelayer, TxInputRef, VerifiedAuxData};
use strata_asm_proto_checkpoint_txs::parser::extract_signed_checkpoint_from_envelope;
use strata_checkpoint_types_ssz::{
    CheckpointClaim, CheckpointPayload, CheckpointScope, CheckpointTip, L1BlockHeightRange,
    L2BlockRange,
};
use strata_crypto::hash;
use strata_identifiers::Epoch;

use crate::{
    errors::{CheckpointError, CheckpointResult},
    state::CheckpointState,
};

/// Processes a checkpoint transaction from L1.
///
/// Validates and verifies a checkpoint submission by performing the following steps:
/// 1. Extracts the signed checkpoint from the transaction envelope
/// 2. Verifies the sequencer signature using the predicate framework
/// 3. Validates that the checkpoint advances to the next expected epoch
/// 4. Constructs a complete checkpoint claim from payload and auxiliary data
/// 5. Verifies the checkpoint proof against the claim using the checkpoint predicate
/// 6. Updates the state with the new verified checkpoint tip
pub(crate) fn handle_checkpoint_tx(
    state: &mut CheckpointState,
    tx: &TxInputRef<'_>,
    verified_aux_data: &VerifiedAuxData,
    relayer: &mut impl MsgRelayer,
) -> CheckpointResult<()> {
    // 1. Extract signed checkpoint from envelope
    let signed_checkpoint = extract_signed_checkpoint_from_envelope(tx)?;
    let checkpoint_payload = &signed_checkpoint.inner;

    // 2. Verify signature using predicate framework.
    // The predicate verifier expects raw payload bytes (not pre-hashed) because
    // BIP-340 Schnorr verification hashes the message internally using tagged hashing.
    let payload_bytes = checkpoint_payload.as_ssz_bytes();
    state
        .sequencer_predicate()
        .verify_claim_witness(&payload_bytes, signed_checkpoint.signature.as_ref())
        .map_err(|_| CheckpointError::InvalidSignature)?;

    // 3. Validate epoch progression
    let expected_epoch = state.verified_tip().epoch + 1;
    if checkpoint_payload.new_tip().epoch != expected_epoch {
        // error
    }

    let claim = construct_full_claim(
        expected_epoch,
        state.verified_tip(),
        checkpoint_payload,
        verified_aux_data,
    )?;

    state
        .checkpoint_predicate()
        .verify_claim_witness(&claim.as_ssz_bytes(), checkpoint_payload.proof())?;

    state.update_verified_tip(checkpoint_payload.new_tip);

    Ok(())
}

/// Constructs a complete checkpoint claim for verification.
fn construct_full_claim(
    epoch: Epoch,
    current_tip: &CheckpointTip,
    payload: &CheckpointPayload,
    verified_aux_data: &VerifiedAuxData,
) -> CheckpointResult<CheckpointClaim> {
    let start_height = current_tip.l1_height() + 1;
    let end_height = payload.new_tip().l1_height();

    let l1_range = L1BlockHeightRange::new(start_height, end_height);
    let l2_range = L2BlockRange::new(
        *current_tip.l2_commitment(),
        payload.new_tip().l2_commitment,
    );
    let scope = CheckpointScope::new(l1_range, l2_range);

    let state_diff_hash = hash::raw(payload.sidecar().ol_state_diff()).into();

    // Convert slice to Vec to enable SSZ encoding, then hash the encoded bytes
    let ol_logs_vec = payload.sidecar().ol_logs().to_vec();
    let ol_logs_hash = hash::raw(&ol_logs_vec.as_ssz_bytes()).into();

    let input_msgs_commitment = compute_input_msgs_commitment(verified_aux_data, l1_range)?;

    Ok(CheckpointClaim::new(
        epoch,
        scope,
        state_diff_hash,
        input_msgs_commitment,
        ol_logs_hash,
    ))
}

/// Computes a cryptographic commitment over L1 manifest hashes.
fn compute_input_msgs_commitment(
    verified_aux_data: &VerifiedAuxData,
    l1_range: L1BlockHeightRange,
) -> CheckpointResult<FixedBytes<32>> {
    // Concatenate all hashes and compute a single hash
    let manifest_hashes =
        verified_aux_data.get_manifest_hashes(l1_range.start as u64, l1_range.end as u64)?;
    let mut data = Vec::with_capacity(manifest_hashes.len() * 32);
    for h in manifest_hashes {
        data.extend_from_slice(h.as_ref());
    }
    let hash = hash::raw(&data);
    Ok(hash.into())
}
