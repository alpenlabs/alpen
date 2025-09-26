use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_crypto::multisig::SchnorrMultisigConfigUpdate;
use strata_primitives::roles::Role;

/// An update to a multisig configuration for a specific role:
/// - adds new members
/// - removes old members
/// - updates the threshold
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct MultisigUpdate {
    config: SchnorrMultisigConfigUpdate,
    role: Role,
}

impl MultisigUpdate {
    /// Create a `MultisigUpdate` with given config and role.
    pub fn new(config: SchnorrMultisigConfigUpdate, role: Role) -> Self {
        Self { config, role }
    }

    /// Borrow the multisig config update.
    pub fn config(&self) -> &SchnorrMultisigConfigUpdate {
        &self.config
    }

    /// Get the role this update applies to.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Consume and return the inner config and role.
    pub fn into_inner(self) -> (SchnorrMultisigConfigUpdate, Role) {
        (self.config, self.role)
    }
}
