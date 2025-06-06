use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;

use crate::{error::UpgradeError, multisig::config::MultisigConfigUpdate, roles::Role};

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

impl MultisigUpdate {
    // Placeholder for actual extraction logic
    pub fn extract_from_tx(_tx: &TxInput<'_>) -> Result<Self, UpgradeError> {
        let action = MultisigUpdate::new(
            MultisigConfigUpdate::new(vec![], vec![], 0),
            Role::BridgeAdmin,
        );
        Ok(action)
    }
}
