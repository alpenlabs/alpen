use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;
use strata_primitives::buf::Buf32;

use super::ActionId;
use crate::{
    error::UpgradeError,
    multisig::{msg::MultisigOp, vote::AggregatedVote},
    state::UpgradeSubprotoState,
};

pub const CANCEL_TX_TYPE: u8 = 0;

#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, BorshSerialize, BorshDeserialize)]
pub struct CancelAction {
    id: ActionId,
}

impl CancelAction {
    pub fn new(id: ActionId) -> Self {
        CancelAction { id }
    }

    pub fn id(&self) -> &ActionId {
        &self.id
    }
}

impl CancelAction {
    /// Extracts a CancelAction from a transaction input.
    /// This is a placeholder function and should be replaced with actual logic.
    pub fn extract_from_tx(tx: &TxInput<'_>) -> Result<Self, UpgradeError> {
        // sanity check
        assert_eq!(tx.tag().tx_type(), CANCEL_TX_TYPE);

        let id = Buf32::zero().into();
        Ok(CancelAction::new(id))
    }
}

/// Handles a CancelAction transaction. It validates the vote on the cancellation
/// and, if valid, removes the specified pending action from the state.
pub fn handle_cancel_tx(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
) -> Result<(), UpgradeError> {
    // Extract the aggregated vote and CancelAction from the transaction payload
    let vote = AggregatedVote::extract_from_tx(tx)?;
    let cancel_action = CancelAction::extract_from_tx(tx)?;

    // Determine the ID of the pending action that should be canceled
    let target_action_id = *cancel_action.id();
    let pending_action = state
        .get_pending_action(&target_action_id)
        .ok_or(UpgradeError::UnknownAction(target_action_id))?;

    // Get the authority that can cancel the pending action
    let role = pending_action.role();
    let authority = state
        .get_authority(&role)
        .ok_or(UpgradeError::UnknownRole)?;

    // Convert the cancel action into a multisig operation and validate it against the vote
    let op = MultisigOp::from(cancel_action);
    authority.validate_op(&op, &vote)?;

    // All checks passed—remove the pending action from the state
    state.remove_pending_action(&target_action_id);

    Ok(())
}
