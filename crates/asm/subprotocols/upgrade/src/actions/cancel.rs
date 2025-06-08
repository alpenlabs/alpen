use strata_asm_common::TxInput;

use crate::{
    error::UpgradeError,
    multisig::{msg::MultisigOp, vote::AggregatedVote},
    state::UpgradeSubprotoState,
    txs::cancel::CancelAction,
};

/// Handles a cancel transaction:
/// 1. Extracts vote and cancel action
/// 2. Validates the vote against the target queued upgrade
/// 3. Removes the queued upgrade from the state
/// 4. Advances the authority nonce
pub fn handle_cancel_tx(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
) -> Result<(), UpgradeError> {
    // Extract the aggregated vote and CancelAction from the transaction payload
    let vote = AggregatedVote::extract_from_tx(tx)?;
    let cancel_action = CancelAction::extract_from_tx(tx)?;

    // Find the pending action that should be canceled
    let target_action_id = *cancel_action.id();
    let queued = state
        .find_queued(&target_action_id)
        .ok_or(UpgradeError::UnknownAction(target_action_id))?;

    // Get the authority that can cancel the pending action
    let role = queued.action().required_role();
    let authority = state.authority(role).ok_or(UpgradeError::UnknownRole)?;

    // Convert the cancel action into a multisig operation and validate it against the vote
    let op = MultisigOp::from(cancel_action);
    authority.validate_op(&op, &vote)?;

    // All checks passed—remove the pending action from the state
    state.remove_queued(&target_action_id);

    // Increase the nonce
    let authority = state.authority_mut(role).ok_or(UpgradeError::UnknownRole)?;
    authority.increment_nonce();

    Ok(())
}
