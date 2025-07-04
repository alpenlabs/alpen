use borsh::BorshDeserialize;
use strata_asm_common::{BRIDGE_SUBPROTOCOL_ID, Log, MessagesContainer, MsgRelayer, TxInput};
use strata_primitives::{
    batch::{SignedCheckpoint, verify_signed_checkpoint_sig},
    block_credential::CredRule,
    buf::Buf32,
    proof::RollupVerifyingKey,
};
use zkaleido::{ProofReceipt, PublicValues};

use crate::{CoreOLState, checkpoint_zk_verifier::*, error::*, utils};

/// Handles OL STF checkpoint transactions according to the specification
///
/// This function implements the complete checkpoint verification workflow:
///
/// 1. **Extract and validate** the signed checkpoint from transaction data
/// 2. **Verify signature** using the current sequencer public key
/// 3. **Verify zk-SNARK proof** using the current verifying key
/// 4. **Construct expected public parameters** from trusted state
/// 5. **Validate state transitions** (epochs, block heights, hashes)
/// 6. **Verify L1→L2 message range** using rolling hash
/// 7. **Update internal state** with new checkpoint summary
/// 8. **Forward withdrawal messages** to Bridge subprotocol
/// 9. **Emit checkpoint summary log** for external monitoring
///
/// # Security Notes
///
/// - Proof public parameters should constructed from our own state, not sequencer input
/// - All state transitions are validated for proper progression
/// - Proof verification uses verifying key from state
/// - L1→L2 message commitments are verified against expected range
pub(crate) fn ol_stf_checkpoint_handler(
    state: &mut CoreOLState,
    tx: &TxInput<'_>,
    relayer: &mut impl MsgRelayer,
) -> Result<()> {
    // Extract signed checkpoint
    let signed_checkpoint = extract_signed_checkpoint(tx)?;

    // Signature Verification
    let cred_rule = CredRule::SchnorrKey(state.sequencer_pubkey);
    if !verify_signed_checkpoint_sig(&signed_checkpoint, &cred_rule) {
        return Err(CoreError::InvalidSignature);
    }

    let checkpoint = signed_checkpoint.checkpoint();

    let public_params = construct_expected_public_parameters(state, checkpoint)?;

    let public_values =
        PublicValues::new(borsh::to_vec(&public_params).expect("checkpoint: proof output"));

    let proof = checkpoint.proof().clone();
    let proof_receipt = ProofReceipt::new(proof, public_values);

    // TODO: Replace with Rollup Verifying Key Framework when available
    // Current implementation is a placeholder that only supports SP1 verifying keys.
    // This should be updated to handle multiple proof systems and key formats.
    let vk_inner = Buf32::try_from_slice(state.checkpoint_vk.as_bytes())
        .map_err(|e| CoreError::InvalidVerifyingKeyFormat(e.to_string()))?;
    let rollup_vk = RollupVerifyingKey::SP1VerifyingKey(vk_inner);

    // Proof Verification
    verify_proof(checkpoint, &proof_receipt, &rollup_vk)?;

    // TODO: Validate L1→L2 Message Range
    // Recompute the rolling hash to verify consistency
    // Waiting on L1→L2 Spec finalization

    // Update State
    state.verified_checkpoint = public_params.epoch_summary;

    // TODO: why we last_checkpoint_ref directly in state? it's already in verified_checkpoint
    state.last_checkpoint_ref = *checkpoint.batch_info().final_l1_block().blkid();

    // Send inter subprotocol messages
    // Validate and forward OL→ASM messages to appropriate subprotocols
    utils::validate_l2_to_l1_messages(&public_params.l2_to_l1_msgs)?;

    if !public_params.l2_to_l1_msgs.is_empty() {
        // Convert OLToASMMessage to Message format and send to bridge
        let mut messages = Vec::new();
        for ol_msg in &public_params.l2_to_l1_msgs {
            if let Ok(decoded) = ol_msg.decode() {
                messages.push(decoded);
            }
        }

        if !messages.is_empty() {
            let container = MessagesContainer::with_messages(BRIDGE_SUBPROTOCOL_ID, messages);
            relayer.relay_msg(&container);
        }
    }

    // Emit Log of the Summary
    // TODO: Emit required log for core subprotocol
    // this is a placeholder implementation
    let summary_body =
        borsh::to_vec(&public_params.epoch_summary).map_err(|_| CoreError::SerializationError)?;
    let log = Log::new(1, summary_body).map_err(|_| CoreError::SerializationError)?;
    relayer.emit_log(log);

    Ok(())
}

/// Extracts a signed checkpoint from a transaction
fn extract_signed_checkpoint(tx: &TxInput<'_>) -> Result<SignedCheckpoint> {
    // TODO: Finalize checkpoint transaction data format specification
    // The specification for auxiliary data in checkpoint transactions is not yet complete.
    // Currently assuming aux_data contains no critical information and using witness data
    // from the first transaction input for demonstration purposes.
    // Update this extraction logic once the specification is finalized.

    let _aux_data = tx.tag().aux_data();

    // TODO: Parse inscription envelope and extract the actual signed checkpoint data
    // For now, we directly use the witness data from the first input as a placeholder.
    let witness_data = tx.tx().input[0].script_sig.as_bytes();

    if witness_data.is_empty() {
        return Err(CoreError::MalformedSignedCheckpoint {
            reason: "witness data is empty".to_string(),
        });
    }

    // The auxiliary data should contain the borsh-serialized SignedCheckpoint
    borsh::from_slice(witness_data).map_err(|e| CoreError::MalformedSignedCheckpoint {
        reason: format!("failed to deserialize SignedCheckpoint: {e}"),
    })
}
