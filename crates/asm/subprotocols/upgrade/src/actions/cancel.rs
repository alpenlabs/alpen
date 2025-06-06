use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};
use strata_primitives::buf::Buf32;

use super::ActionId;
use crate::{
    crypto::Signature, error::UpgradeError, state::UpgradeSubprotoState, vote::AggregatedVote,
};

pub const CANCEL_TX_TYPE: u8 = 5;

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

/// Handles a CancelAction transaction. It validates the vote on the cancellation
/// and, if valid, removes the specified pending action from the state.
pub fn handle_cancel_action(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    // Extract the CancelAction and its accompanying vote from the transaction
    let (cancel_action, vote) = extract_cancel_action(tx)?;

    // Determine the ID of the pending action that should be canceled
    let target_action_id = *cancel_action.id();
    let pending_action = state
        .get_pending_action(&target_action_id)
        .ok_or(UpgradeError::UnknownAction(target_action_id))?;

    // Fetch the multisig authority configuration for the role associated with the pending action
    let role = *pending_action.role();
    let multisig_config = state
        .get_multisig_authority_config(&role)
        .ok_or(UpgradeError::UnknownRole)?;

    // Validate the cancel action
    vote.validate_action(&multisig_config.keys, &cancel_action.into())?;

    // All checks passed—remove the pending action from the state
    state.remove_pending_action(&target_action_id);

    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_cancel_action(tx: &TxInput<'_>) -> Result<(CancelAction, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), CANCEL_TX_TYPE);

    let id = Buf32::zero().into();
    let action = CancelAction::new(id);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());
    Ok((action, vote))
}
