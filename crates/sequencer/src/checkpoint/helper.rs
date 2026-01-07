//! reusable utils.

use k256::schnorr::{signature::Verifier, Signature, VerifyingKey};
use ssz::Encode;
use strata_checkpoint_types::Checkpoint;
use strata_checkpoint_types_ssz::{
    BatchInfo as SszBatchInfo, CheckpointCommitment, CheckpointPayload, CheckpointSidecar,
    L1BlockRange, L2BlockRange, SignedCheckpointPayload,
};
use strata_codec::encode_to_vec;
use strata_identifiers::OLBlockCommitment;
use strata_ol_chain_types_new::{OLLog, SimpleWithdrawalIntentLogData};
use strata_ol_chainstate_types::Chainstate;
use strata_ol_stf::BRIDGE_GATEWAY_ACCT_SERIAL;
use strata_params::Params;
use strata_primitives::{block_credential::CredRule, buf::Buf64};
use tracing::warn;

/// Verify signature over the SSZ CheckpointPayload.
pub fn verify_checkpoint_payload_sig(
    payload: &CheckpointPayload,
    signature: &Buf64,
    params: &Params,
) -> bool {
    // Get the sequencer public key from params
    let pubkey = match &params.rollup().cred_rule {
        CredRule::Unchecked => return true,
        CredRule::SchnorrKey(pk) => pk,
    };

    // Parse the verifying key from the sequencer public key
    let Ok(verifying_key) = VerifyingKey::from_bytes(pubkey.as_ref()) else {
        return false;
    };

    // Parse the signature (64-byte BIP-340 signature)
    let sig_bytes: &[u8] = signature.as_ref();
    let Ok(sig) = Signature::try_from(sig_bytes) else {
        return false;
    };

    // Verify against raw SSZ bytes (k256 handles BIP-340 tagged hashing internally)
    let payload_bytes = payload.as_ssz_bytes();
    verifying_key.verify(&payload_bytes, &sig).is_ok()
}

/// Verify signature on a SignedCheckpointPayload.
pub fn verify_signed_checkpoint_payload(
    signed_payload: &SignedCheckpointPayload,
    params: &Params,
) -> bool {
    verify_checkpoint_payload_sig(&signed_payload.inner, &signed_payload.signature, params)
}

/// Converts an `Checkpoint-v0` to the SSZ `CheckpointPayload` format.
pub fn convert_checkpoint_to_payload(checkpoint: &Checkpoint) -> CheckpointPayload {
    let batch_info = checkpoint.batch_info();
    let batch_transition = checkpoint.batch_transition();

    // Convert L1 range using L1BlockCommitment directly
    let l1_range = L1BlockRange::new(batch_info.l1_range.0, batch_info.l1_range.1);

    // Convert L2 range - map L2BlockCommitment to OLBlockCommitment
    let l2_start =
        OLBlockCommitment::new(batch_info.l2_range.0.slot(), *batch_info.l2_range.0.blkid());
    let l2_end =
        OLBlockCommitment::new(batch_info.l2_range.1.slot(), *batch_info.l2_range.1.blkid());
    let l2_range = L2BlockRange::new(l2_start, l2_end);

    // Create batch info
    let ssz_batch_info = SszBatchInfo::new(batch_info.epoch, l1_range, l2_range);

    // Create commitment with post-state root (pre-state derived from ASM state during verification)
    let post_state_root = batch_transition.chainstate_transition.post_state_root;
    let commitment = CheckpointCommitment::new(ssz_batch_info, post_state_root);

    // Extract OL logs from the chainstate's pending_withdraws.
    // The checkpoint-v0 sidecar contains serialized chainstate which includes pending withdrawal
    // intents. We convert these to the OL log format expected by the checkpoint subprotocol.
    let ol_logs = extract_ol_logs_from_chainstate(checkpoint.sidecar().chainstate());

    let sidecar = CheckpointSidecar::new(
        vec![],
        ol_logs, // ol_logs - extracted from chainstate pending_withdraws
    )
    .expect("sidecar should be valid");

    // Get proof bytes
    let proof_bytes = checkpoint.proof().as_bytes().to_vec();

    CheckpointPayload::new(commitment, sidecar, proof_bytes)
        .expect("checkpoint payload construction should succeed")
}

/// Extracts withdrawal intents from chainstate and converts them to SSZ-encoded OL logs.
///
/// The checkpoint-v0 format stores a serialized `Chainstate` in the sidecar, which contains
/// `pending_withdraws` - a queue of withdrawal intents. This function:
/// 1. Deserializes the chainstate from borsh bytes
/// 2. Extracts each `WithdrawalIntent` from `pending_withdraws`
/// 3. Converts each to `SimpleWithdrawalIntentLogData` (the OL log format)
/// 4. Wraps each in an `OLLog` with the bridge gateway account serial
/// 5. SSZ-encodes the resulting `Vec<OLLog>`
///
/// This bridges the legacy checkpoint format with the SPS-62 checkpoint subprotocol
/// which expects withdrawal intents as OL logs in the sidecar.
fn extract_ol_logs_from_chainstate(chainstate_bytes: &[u8]) -> Vec<u8> {
    // Deserialize chainstate from borsh bytes
    let chainstate: Chainstate = match borsh::from_slice(chainstate_bytes) {
        Ok(cs) => cs,
        Err(e) => {
            warn!(error = %e, "Failed to deserialize chainstate for OL log extraction");
            return vec![];
        }
    };

    // Convert each WithdrawalIntent to an OLLog
    let logs: Vec<OLLog> = chainstate
        .pending_withdraws()
        .entries()
        .iter()
        .filter_map(|intent| {
            // Create the log payload with amount (sats) and destination (BOSD bytes)
            let log_data = SimpleWithdrawalIntentLogData::new(
                intent.amt().to_sat(),
                intent.destination().to_bytes().to_vec(),
            )?;

            // Encode the log payload using strata-codec
            let encoded = match encode_to_vec(&log_data) {
                Ok(bytes) => bytes,
                Err(e) => {
                    warn!(error = %e, "Failed to encode withdrawal intent log data");
                    return None;
                }
            };

            // Wrap in OLLog with the bridge gateway account serial
            Some(OLLog::new(BRIDGE_GATEWAY_ACCT_SERIAL, encoded))
        })
        .collect();

    // SSZ encode the logs vector (empty vec encodes to empty bytes)
    if logs.is_empty() {
        vec![]
    } else {
        logs.as_ssz_bytes()
    }
}
