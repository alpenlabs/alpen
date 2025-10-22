//! OL STF Checkpoint transaction handler
//!
//! Handles checkpoint verification and state updates for the Core subprotocol.

use strata_asm_common::{AnchorState, AuxResolver, MsgRelayer, TxInputRef};

use crate::{CoreOLState, error::*, messages, parsing, types, verification};

/// Handles OL STF checkpoint transactions according to the specification
///
/// This function implements the complete checkpoint verification workflow:
///
/// 1. **Extract and validate** the signed checkpoint from transaction data
/// 2. **Verify signature** using the current sequencer public key
/// 3. **Verify Checkpoint zk-SNARK proof** using the current verifying key
/// 4. **Construct expected public parameters** from local ASM state
/// 5. **Validate state transitions** (epochs, block heights, hashes)
/// 6. **Verify L1→L2 message range** using rolling hash
/// 7. **Update internal state** with new checkpoint summary
/// 8. **Forward withdrawal messages** to Bridge subprotocol
/// 9. **Emit logs** for OL (TODO: define the log format)
///
/// [PLACE_HOLDER] => What are the rest of security checks that are not covered by these steps?
/// [PLACE_HOLDER] => Define the role of anchor_pre and aux_inputs in checkpoint validation logic
/// [PLACE_HOLDER] => Define messages type that we gonna send to other subprotocols
///                   (e.g. withdrawal, etc.)
///
///
/// # Security Notes
///
/// - Proof public parameters should constructed from our own state, not sequencer input
/// - All state transitions are validated for proper progression
/// - Proof verification uses verifying key from state
/// - L1→L2 message commitments are verified against expected range
pub(crate) fn handle_checkpoint_transaction(
    state: &mut CoreOLState,
    tx: &TxInputRef<'_>,
    _relayer: &mut impl MsgRelayer,
    anchor_pre: &AnchorState,
    // TODO make this the actual type
    aux_resolver: &dyn AuxResolver,
) -> Result<()> {
    // 1. Extract and validate signed checkpoint
    let signed_checkpoint = parsing::extract_signed_checkpoint(tx)?;

    // 2. Verify signature using dedicated signature verification function
    verification::verify_checkpoint_signature(&signed_checkpoint, &state.sequencer_pubkey)?;

    let checkpoint = signed_checkpoint.checkpoint();

    // 3. Validate state transition before processing
    verification::validate_state_transition(state, checkpoint)?;

    // 4. Construct expected public parameters from trusted state
    let public_params =
        verification::construct_checkpoint_proof_public_parameters(state, checkpoint)?;

    // 5. Construct raw public values
    // FIXME: This should return an error instead of panicking
    let public_values = borsh::to_vec(&public_params).expect("checkpoint: proof output");

    // 7. Verify the zk-SNARK proof
    state
        .checkpoint_predicate()
        .verify_claim_witness(&public_values, checkpoint.proof().as_bytes())?;

    // 8. Validate L1→L2 Message Range using the rolling hash
    let prev_l1_height = state.verified_checkpoint.new_l1().height_u64();
    let new_l1_height = checkpoint.batch_info().final_l1_block().height_u64();
    let expected_commitment = &public_params.l1_to_l2_msgs_range_commitment_hash;
    messages::validate_l1_to_l2_messages(
        prev_l1_height,
        new_l1_height,
        expected_commitment,
        anchor_pre,
        aux_resolver,
    )?;

    // 9. Validate L2→L1 messages
    messages::validate_l2_to_l1_messages(&public_params.l2_to_l1_msgs)?;

    // 10. Apply checkpoint to state using dedicated state management function
    types::apply_checkpoint_to_state(state, public_params.epoch_summary, checkpoint);

    // [PLACE_HOLDER] => Update here when we have the design of L2 → L1 messaging system.
    // 11. Forward withdrawal messages to Bridge subprotocol
    // [PLACE_HOLDER] TODO: Fix inter-protocol messaging
    // Key points:
    // - Don't pass raw OL logs as inter-proto messages
    // - Bridge subprotocol should export opaque enum types for messages it expects
    // - Each subprotocol should define its own message interface
    // - Use typed messages instead of raw OwnedMsg objects
    // - Example: BridgeMessage::Withdrawal { recipient, amount }
    /*
    if !public_params.l2_to_l1_msgs.is_empty() {
        // Convert Message to OwnedMsg format and send to bridge
        // [PLACE_HOLDER] Update the names to align with the team's new naming convention.
        let mut bridge_messages = Vec::new();
        for ol_msg in &public_params.l2_to_l1_msgs {
            bridge_messages.push(ol_msg.to_msg());
        }

        if !bridge_messages.is_empty() {
            let container =
                MessagesContainer::with_messages(BRIDGE_SUBPROTOCOL_ID, bridge_messages);
            relayer.relay_msg(&container);
        }
    }
    */

    // 12. Emit Log of the Summary
    // [PLACE_HOLDER]
    // TODO: Emit required log for core subprotocol
    // For now, we'll skip log emission to avoid dependency issues
    // This can be implemented later when the proper log structure is defined
    let _summary_body =
        borsh::to_vec(&public_params.epoch_summary).map_err(|_| CoreError::SerializationError)?;

    // TODO: Create and emit proper log entry once log format is finalized
    // relayer.emit_log(log_entry);

    Ok(())
}
