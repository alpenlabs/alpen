//! Basic containers for tracking OL state changes across blocks.

use strata_ol_chainstate_types::Chainstate;

/// Description of the basic writes we need to make to the underlying database.
///
/// This is NOT meant to be a full DA diff.  There may be redundant information
/// in here that a proper DA diff does not need.
// This is kinda redundant compared to `WriteBatch`, so it's being removed.
#[derive(Clone, Debug)]
pub struct ChangedState {
    toplevel: Chainstate,
}

impl ChangedState {
    pub fn new(toplevel: Chainstate) -> Self {
        Self { toplevel }
    }

    pub fn toplevel(&self) -> &Chainstate {
        &self.toplevel
    }
}
