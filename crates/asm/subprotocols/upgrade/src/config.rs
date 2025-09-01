use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_crypto::multisig::config::MultisigConfig;
use strata_primitives::roles::Role;

/// Configuration for the upgrade subprotocol, containing MultisigConfig for each role.
///
/// Design choice: Uses individual named fields rather than `Vec<(Role, MultisigConfig)>`
/// to ensure structural completeness - the compiler guarantees all 4 config fields are
/// provided when constructing this struct. However, it does NOT prevent logical errors
/// like using the same config for multiple roles or mismatched role-field assignments.
/// The benefit is avoiding missing fields at compile-time rather than runtime validation.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct UpgradeSubprotoConfig {
    /// MultisigConfig for BridgeAdmin role
    pub bridge_admin: MultisigConfig,
    /// MultisigConfig for BridgeConsensusManager role
    pub bridge_consensus_manager: MultisigConfig,
    /// MultisigConfig for StrataAdmin role
    pub strata_admin: MultisigConfig,
    /// MultisigConfig for StrataConsensusManager role
    pub strata_consensus_manager: MultisigConfig,
}

impl UpgradeSubprotoConfig {
    pub fn new(
        bridge_admin: MultisigConfig,
        bridge_consensus_manager: MultisigConfig,
        strata_admin: MultisigConfig,
        strata_consensus_manager: MultisigConfig,
    ) -> Self {
        Self {
            bridge_admin,
            bridge_consensus_manager,
            strata_admin,
            strata_consensus_manager,
        }
    }

    pub fn get_config(&self, role: Role) -> &MultisigConfig {
        match role {
            Role::BridgeAdmin => &self.bridge_admin,
            Role::BridgeConsensusManager => &self.bridge_consensus_manager,
            Role::StrataAdmin => &self.strata_admin,
            Role::StrataConsensusManager => &self.strata_consensus_manager,
        }
    }

    pub fn get_all_authorities(self) -> Vec<(Role, MultisigConfig)> {
        vec![
            (Role::BridgeAdmin, self.bridge_admin),
            (Role::BridgeConsensusManager, self.bridge_consensus_manager),
            (Role::StrataAdmin, self.strata_admin),
            (Role::StrataConsensusManager, self.strata_consensus_manager),
        ]
    }
}
