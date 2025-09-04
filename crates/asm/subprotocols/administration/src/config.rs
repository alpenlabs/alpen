use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_crypto::multisig::config::MultisigConfig;
use strata_primitives::roles::Role;

/// Parameters for the admnistration subprotocol, containing MultisigConfig for each role.
///
/// Design choice: Uses individual named fields rather than `Vec<(Role, MultisigConfig)>`
/// to ensure structural completeness - the compiler guarantees all 4 config fields are
/// provided when constructing this struct. However, it does NOT prevent logical errors
/// like using the same config for multiple roles or mismatched role-field assignments.
/// The benefit is avoiding missing fields at compile-time rather than runtime validation.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct AdministrationSubprotoParams {
    /// MultisigConfig for [StrataAdministrator](Role::StrataAdministrator).
    pub strata_administrator: MultisigConfig,

    /// MultisigConfig for [StrataSequencerManager](Role::StrataSequencerManager).
    pub strata_sequencer_manager: MultisigConfig,

    /// The confirmation depth (CD) setting: after an update transaction receives this many
    /// confirmations, the update is enacted automatically. During this confirmation period,
    /// the update can still be cancelled by submitting a cancel transaction.
    pub confirmation_depth: u32,
}

impl AdministrationSubprotoParams {
    pub fn new(
        strata_administrator: MultisigConfig,
        strata_sequencer_manager: MultisigConfig,
        confirmation_depth: u32,
    ) -> Self {
        Self {
            strata_administrator,
            strata_sequencer_manager,
            confirmation_depth,
        }
    }

    pub fn get_config(&self, role: Role) -> &MultisigConfig {
        match role {
            Role::StrataAdministrator => &self.strata_administrator,
            Role::StrataSequencerManager => &self.strata_sequencer_manager,
        }
    }

    pub fn get_all_authorities(self) -> Vec<(Role, MultisigConfig)> {
        vec![
            (Role::StrataAdministrator, self.strata_administrator),
            (Role::StrataSequencerManager, self.strata_sequencer_manager),
        ]
    }
}
