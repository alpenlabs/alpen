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
};

/// Handles an incoming upgrade transaction:
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

    // Retrieve the authority entity responsible for this action's role
    let authority = state
        .get_authority(&action.role())
        .ok_or(UpgradeError::UnknownRole)?;

    // If this is a multisig configuration update, ensure the new config is valid
    if let UpgradeAction::Multisig(update) = &action {
        authority.config().validate_update(update.config_update())?;
    }

    // Convert the action into a multisig operation and validate it against the vote
    let op = MultisigOp::from(action.clone());
    authority.validate_op(&op, &vote)?;

    // Create a pending upgrade action and enqueue it for later enactment
    let pending_action = super::PendingUpgradeAction::from(action);
    state.add_pending_action(pending_action);

    Ok(())
}
