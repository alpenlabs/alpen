use borsh::{BorshDeserialize, BorshSerialize};
use strata_ol_chainstate_types::Chainstate;

/// Describes the entry for a state in the database.
#[derive(Clone, Debug, Eq, PartialEq, BorshDeserialize, BorshSerialize)]
pub struct StateInstanceEntry {
    pub(crate) toplevel_state: Chainstate,
}

impl StateInstanceEntry {
    pub fn new(toplevel_state: Chainstate) -> Self {
        Self { toplevel_state }
    }

    pub fn toplevel_state(&self) -> &Chainstate {
        &self.toplevel_state
    }

    pub fn into_toplevel_state(self) -> Chainstate {
        self.toplevel_state
    }
}
