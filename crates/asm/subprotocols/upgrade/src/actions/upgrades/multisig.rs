use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use crate::{
    actions::PendingUpgradeAction,
    crypto::{PubKey, Signature},
    error::UpgradeError,
    multisig::vote::AggregatedVote,
    roles::Role,
    state::UpgradeSubprotoState,
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
}

pub fn handle_multisig_config_update_tx(
    state: &mut UpgradeSubprotoState,
    tx: &TxInput<'_>,
    _relayer: &mut impl MsgRelayer,
) -> Result<(), UpgradeError> {
    // Extract multisig config update and vote
    let (update, vote) = extract_multisig_update(tx)?;

    // Fetch current multisig configuration
    let role = *update.role();
    let existing_config = state
        .get_multisig_config(&role)
        .ok_or(UpgradeError::UnknownRole)?; // TODO: better error name

    // validate config update
    existing_config.validate_update(&update)?;

    // Validate the vote for the action
    let action = update.into();
    vote.validate_action(&existing_config.keys, &action)?;

    // Create the pending action and enqueue it
    let pending_action =
        PendingUpgradeAction::new(action, MULTISIG_CONFIG_UPDATE_ENACTMENT_DELAY, role);
    state.add_pending_action(pending_action);

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
