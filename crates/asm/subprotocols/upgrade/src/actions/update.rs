use strata_asm_common::TxInput;

use crate::{
    error::UpgradeError,
    multisig::{msg::MultisigOp, vote::AggregatedVote},
    state::UpgradeSubprotoState,
    txs::{
        MULTISIG_CONFIG_UPDATE_TX_TYPE, OPERATOR_UPDATE_TX_TYPE, SEQUENCER_UPDATE_TX_TYPE,
        VK_UPDATE_TX_TYPE,
        updates::{
            UpgradeAction, multisig::MultisigUpdate, operator::OperatorSetUpdate,
            seq::SequencerUpdate, vk::VerifyingKeyUpdate,
        },
    },
    upgrades::{queued::QueuedUpgrade, scheduled::ScheduledUpgrade},
};

/// Handles an incoming upgrade transaction:
/// 1. Extracts vote and action
/// 2. Validates multisig config if needed
/// 3. Verifies vote signature
/// 4. Queues or schedules the action
/// 5. Advances the authority nonce
pub fn handle_update_tx(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
) -> Result<(), UpgradeError> {
    // Extract the aggregated vote from the transaction payload
    let vote = AggregatedVote::extract_from_tx(tx)?;

    // Parse the transaction into a concrete UpgradeAction based on its type tag
    let action: UpgradeAction = match tx.tag().tx_type() {
        VK_UPDATE_TX_TYPE => {
            // Extract a VerifyingKeyUpdate and wrap it
            Ok(UpgradeAction::from(VerifyingKeyUpdate::extract_from_tx(
                tx,
            )?))
        }
        SEQUENCER_UPDATE_TX_TYPE => {
            // Extract a SequencerUpdate and wrap it
            Ok(UpgradeAction::from(SequencerUpdate::extract_from_tx(tx)?))
        }
        MULTISIG_CONFIG_UPDATE_TX_TYPE => {
            // Extract a MultisigUpdate and wrap it
            Ok(UpgradeAction::from(MultisigUpdate::extract_from_tx(tx)?))
        }
        OPERATOR_UPDATE_TX_TYPE => {
            // Extract an OperatorSetUpdate and wrap it
            Ok(UpgradeAction::from(OperatorSetUpdate::extract_from_tx(tx)?))
        }
        // Unknown transaction type: cannot determine the upgrade action
        _ => Err(UpgradeError::UnknownRole),
    }?;

    let role = action.required_role();

    // Retrieve the authority entity responsible for this action's role
    let authority = state.authority_mut(role).ok_or(UpgradeError::UnknownRole)?;

    // If this is a multisig configuration update, ensure the new config is valid
    if let UpgradeAction::Multisig(update) = &action {
        authority.config().validate_update(update.config())?;
    }

    // Convert the action into a multisig operation and validate it against the vote
    let op = MultisigOp::from(action.clone());
    authority.validate_op(&op, &vote)?;

    // Queue or schedule the upgrade
    match action {
        // If the action is a VerifyingKeyUpdate, queue it so support cancellation
        UpgradeAction::VerifyingKey(_) => {
            let queued_upgrade: QueuedUpgrade = action.try_into()?;
            state.enqueue(queued_upgrade);
        }
        // For all other actions, directly schedule them for execution
        _ => {
            let scheduled_upgrade: ScheduledUpgrade = action.try_into()?;
            state.schedule(scheduled_upgrade);
        }
    }

    // Increase the nonce
    let authority = state.authority_mut(role).ok_or(UpgradeError::UnknownRole)?;
    authority.increment_nonce();

    Ok(())
}
