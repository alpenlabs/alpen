use strata_asm_common::TxInput;

use crate::{
    error::UpgradeError,
    multisig::{msg::MultisigOp, vote::AggregatedVote},
    state::UpgradeSubprotoState,
    txs::enact::EnactAction,
};

/// Handles an enactment transaction:
/// 1. Extracts vote and enact action
/// 2. Validates the vote against the target committed upgrade
/// 3. Moves the upgrade from committed to scheduled
/// 4. Advances the authority nonce
pub fn handle_enactment_tx(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
) -> Result<(), UpgradeError> {
    // Extract the aggregated vote and CancelAction from the transaction payload
    let vote = AggregatedVote::extract_from_tx(tx)?;
    let enact_action = EnactAction::extract_from_tx(tx)?;

    // Determine the ID of the pending action that should be canceled
    let target_action_id = *enact_action.id();
    let upgrade = state
        .find_committed(&target_action_id)
        .ok_or(UpgradeError::UnknownAction(target_action_id))?;

    // Get the authority that can enact the committed action
    let role = upgrade.action().required_role();
    let authority = state.authority(role).ok_or(UpgradeError::UnknownRole)?;

    // Convert the enact action into a multisig operation and validate it against the vote
    let op = MultisigOp::from(enact_action);
    authority.validate_op(&op, &vote)?;

    // All checks passed - commit to schedule
    state.commit_to_schedule(&target_action_id);

    // Increase the nonce
    let authority = state.authority_mut(role).ok_or(UpgradeError::UnknownRole)?;
    authority.increment_nonce();

    Ok(())
}
