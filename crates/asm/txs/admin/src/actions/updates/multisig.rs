use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_crypto::threshold_signature::ThresholdConfigUpdate;
use strata_primitives::roles::Role;

/// An update to a threshold configuration for a specific role:
/// - adds new members
/// - removes old members
/// - updates the threshold
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct MultisigUpdate {
    config: ThresholdConfigUpdate,
    role: Role,
}

impl MultisigUpdate {
    /// Create a `MultisigUpdate` with given config and role.
    pub fn new(config: ThresholdConfigUpdate, role: Role) -> Self {
        Self { config, role }
    }

    /// Borrow the threshold config update.
    pub fn config(&self) -> &ThresholdConfigUpdate {
        &self.config
    }

    /// Get the role this update applies to.
    pub fn role(&self) -> Role {
        self.role
    }

    /// Consume and return the inner config and role.
    pub fn into_inner(self) -> (ThresholdConfigUpdate, Role) {
        (self.config, self.role)
    }
}
