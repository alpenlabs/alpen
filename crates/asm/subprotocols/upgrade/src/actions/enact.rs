use strata_asm_common::TxInput;

use crate::{
    error::UpgradeError,
    multisig::{msg::MultisigOp, vote::AggregatedVote},
    state::UpgradeSubprotoState,
    txs::enact::EnactAction,
};

/// Handles a CancelAction transaction. It validates the vote on the cancellation
/// and, if valid, removes the specified pending action from the state.
pub fn handle_enactment_tx(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
) -> Result<(), UpgradeError> {
    // Extract the aggregated vote and CancelAction from the transaction payload
    let vote = AggregatedVote::extract_from_tx(tx)?;
    let enact_action = EnactAction::extract_from_tx(tx)?;

    // Determine the ID of the pending action that should be canceled
    let target_action_id = *enact_action.id();
    let pending_action = state
        .get_scheduled_upgrade(&target_action_id)
        .ok_or(UpgradeError::UnknownAction(target_action_id))?;

    // Get the authority that can enact the committed action
    let role = pending_action.action().role();
    let authority = state
        .get_authority(&role)
        .ok_or(UpgradeError::UnknownRole)?;

    // Convert the enact action into a multisig operation and validate it against the vote
    let op = MultisigOp::from(enact_action);
    authority.validate_op(&op, &vote)?;

    // All checks passed—remove the pending action from the state
    state.remove_queued_upgrade(&target_action_id);

    // Increase the nonce
    let authority = state
        .get_authority_mut(&role)
        .ok_or(UpgradeError::UnknownRole)?;
    authority.increment_nonce();

    Ok(())
}
