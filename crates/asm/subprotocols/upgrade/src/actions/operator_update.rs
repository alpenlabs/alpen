use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};
use strata_primitives::hash::compute_borsh_hash;

use crate::{
    actions::{ActionId, PendingUpgradeAction},
    crypto::{PubKey, Signature},
    error::UpgradeError,
    roles::Role,
    state::UpgradeSubprotoState,
    vote::AggregatedVote,
};

pub const OPERATOR_UPDATE_TX_TYPE: u8 = 3;

pub const OPERATOR_UPDATE_ENACTMENT_DELAY: u64 = 2016;

/// Represents a change to the Bridge Operator Set`
/// * removes the specified `old_members` from the set
/// * adds the specified `new_members`
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct OperatorSetUpdate {
    new_members: Vec<PubKey>,
    old_members: Vec<PubKey>,
}

impl OperatorSetUpdate {
    pub fn new(new_members: Vec<PubKey>, old_members: Vec<PubKey>) -> Self {
        Self {
            new_members,
            old_members,
        }
    }

    pub fn compute_action_id(&self) -> ActionId {
        compute_borsh_hash(&self).into()
    }
}

pub fn handle_operator_update(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    // Extract operator update and vote
    let (update, vote) = extract_operator_update(tx)?;

    // BridgeAdmin has the exclusive authority to update bridge operators
    let role = Role::BridgeAdmin;

    // Fetch multisig configuration
    let config = state
        .get_multisig_authority_config(&role)
        .ok_or(UpgradeError::UnknownRole)?; // TODO: better error name

    // Compute ActionId and validate the vote for the action
    let update_action_id = update.compute_action_id();
    vote.validate_action(&config.keys, &update_action_id)?;

    // Create the pending action and enqueue it
    let pending_action =
        PendingUpgradeAction::new(update.into(), OPERATOR_UPDATE_ENACTMENT_DELAY, role);
    state.add_pending_action(update_action_id, pending_action);

    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_operator_update(
    tx: &TxInput<'_>,
) -> Result<(OperatorSetUpdate, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), OPERATOR_UPDATE_TX_TYPE);

    let action = OperatorSetUpdate::new(vec![], vec![]);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());

    Ok((action, vote))
}
