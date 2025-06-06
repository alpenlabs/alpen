use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::{MsgRelayer, TxInput};

use crate::{
    actions::PendingUpgradeAction,
    crypto::Signature,
    error::UpgradeError,
    multisig::{config::MultisigConfigUpdate, vote::AggregatedVote},
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
pub struct MultisigUpdate {
    update: MultisigConfigUpdate,
    role: Role,
}

impl MultisigUpdate {
    pub fn new(update: MultisigConfigUpdate, role: Role) -> Self {
        Self { update, role }
    }

    pub fn config_update(&self) -> &MultisigConfigUpdate {
        &self.update
    }

    pub fn role(&self) -> Role {
        self.role
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
    let role = update.role();
    let existing_config = state
        .get_multisig_config(&role)
        .ok_or(UpgradeError::UnknownRole)?; // TODO: better error name

    // validate config update
    existing_config.validate_update(update.config_update())?;

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
) -> Result<(MultisigUpdate, AggregatedVote), UpgradeError> {
    // sanity check
    assert_eq!(tx.tag().tx_type(), MULTISIG_CONFIG_UPDATE_TX_TYPE);

    let config = MultisigConfigUpdate::new(vec![], vec![], 0);
    let action = MultisigUpdate::new(config, Role::BridgeAdmin);
    let vote = AggregatedVote::new(vec![0u8; 15], Signature::default());

    Ok((action, vote))
}
