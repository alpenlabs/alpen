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

pub const MULTISIG_CONFIG_UPDATE_TX_TYPE: u8 = 1;

const MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY: u64 = 2016;

/// Represents a change to the multisig configuration for the given `role`:
/// * removes the specified `old_members` from the set,
/// * adds the specified `new_members`
/// * updates the threshold.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigConfigUpdate {
    new_members: Vec<PubKey>,
    old_members: Vec<PubKey>,
    new_threshold: u8,
    role: Role,
}

impl MultisigConfigUpdate {
    pub fn new(
        new_members: Vec<PubKey>,
        old_members: Vec<PubKey>,
        new_threshold: u8,
        role: Role,
    ) -> Self {
        Self {
            new_members,
            old_members,
            new_threshold,
            role,
        }
    }

    pub fn old_members(&self) -> &[PubKey] {
        &self.old_members
    }

    pub fn new_members(&self) -> &[PubKey] {
        &self.new_members
    }

    pub fn new_threshold(&self) -> u8 {
        self.new_threshold
    }

    pub fn role(&self) -> &Role {
        &self.role
    }

    pub fn compute_action_id(&self) -> ActionId {
        compute_borsh_hash(&self).into()
    }
}

pub fn handle_multisig_config_update(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    // Extract multisig config update and vote
    let (update, vote) = extract_multisig_update(tx)?;

    // Fetch current multisig configuration
    let role = *update.role();
    let existing_config = state
        .get_multisig_authority_config(&role)
        .ok_or(UpgradeError::UnknownRole)?; // TODO: better error name

    // validate config update
    existing_config.validate_update(&update)?;

    // Compute ActionId and validate the vote for the action
    let update_action_id = update.compute_action_id();
    vote.validate_action(&existing_config.keys, &update_action_id)?;

    // Create the pending action and enqueue it
    let pending_action =
        PendingUpgradeAction::new(update.into(), MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY, role);
    state.add_pending_action(update_action_id, pending_action);

    Ok(())
}

// FIXME: This is a placeholder for now
fn extract_multisig_update(
    tx: &TxInput<'_>,
) -> Result<(MultisigConfigUpdate, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), MULTISIG_CONFIG_UPDATE_TX_TYPE);

    let action = MultisigConfigUpdate::new(vec![], vec![], 0, Role::BridgeAdmin);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());

    Ok((action, vote))
}
