use std::collections::HashMap;

use borsh::{BorshDeserialize, BorshSerialize};

use crate::{
    actions::{ActionId, PendingUpgradeAction},
    roles::Role,
};

/// Holds the state for the upgrade subprotocol, including the various
/// multisignature authorities and any actions still pending execution.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct UpgradeSubprotoState {
    /// Role-specific configuration for a multisignature authority: who the
    /// signers are, and how many signatures are required to approve an action.
    multisig_authority: HashMap<Role, MultisigConfig<u32>>,

    /// A map from each action’s unique identifier to its corresponding
    /// upgrade action awaiting execution.
    pending_actions: HashMap<ActionId, PendingUpgradeAction>,
}

/// Configuration for a multisignature authority: who the signers are, and
/// how many signatures are required to approve an action.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigConfig<T> {
    /// The public keys of all grant-holders authorized to sign.
    pub keys: Vec<T>,
    /// The minimum number of keys that must sign to approve an action.
    pub threshold: u8,
}
