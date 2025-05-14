use strata_state::chain_state::Chainstate;

use crate::context::StateAccessor;

/// Container that tracks writes on top of a database handle for the state we're
/// building on top of.
pub struct State<S: StateAccessor> {
    accessor: S,

    new_chainstate: Chainstate,
}

impl<S: StateAccessor> State<S> {
    /// Constructs a new instance wrapping a previous state.
    pub fn new(accessor: S, new_chainstate: Chainstate) -> Self {
        Self {
            accessor,
            new_chainstate,
        }
    }

    pub fn cur_chainstate(&self) -> &Chainstate {
        &self.new_chainstate
    }
}
