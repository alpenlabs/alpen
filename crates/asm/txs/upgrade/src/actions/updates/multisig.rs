use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInputRef;
use strata_primitives::roles::Role;

use crate::{crypto::multisig_config::MultisigConfigUpdate, error::UpgradeTxParseError};

/// An update to a multisig configuration for a specific role:
/// - adds new members
/// - removes old members
/// - updates the threshold
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct MultisigUpdate {
    config: MultisigConfigUpdate,
    role: Role,
}

impl MultisigUpdate {
    /// Create a `MultisigUpdate` with given config and role.
    pub fn new(config: MultisigConfigUpdate, role: Role) -> Self {
        Self { config, role }
    }

    /// Borrow the multisig config update.
    pub fn config(&self) -> &MultisigConfigUpdate {
        &self.config
    }

    /// Get the role this update applies to.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Consume and return the inner config and role.
    pub fn into_inner(self) -> (MultisigConfigUpdate, Role) {
        (self.config, self.role)
    }

    /// Extract a `MultisigUpdate` from a transaction input.
    ///
    /// Placeholder: replace with actual parsing logic.
    pub fn extract_from_tx(_tx: &TxInputRef<'_>) -> Result<Self, UpgradeTxParseError> {
        // TODO: parse TxInput to build MultisigConfigUpdate and determine Role
        Ok(Self::new(
            MultisigConfigUpdate::new(vec![], vec![], 0),
            Role::BridgeAdmin,
        ))
    }
}
