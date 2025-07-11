use strata_state::chain_state::Chainstate;

use crate::context::StateAccessor;

/// Container that tracks writes on top of a database handle for the state we're
/// building on top of.
#[derive(Debug)]
pub struct State<S: StateAccessor> {
    accessor: S,

    chainstate: Chainstate,
}

impl<S: StateAccessor> State<S> {
    /// Constructs a new instance wrapping a previous state.
    pub fn new(accessor: S, chainstate: Chainstate) -> Self {
        Self {
            accessor,
            chainstate,
        }
    }

    pub fn cur_chainstate(&self) -> &Chainstate {
        &self.chainstate
    }

    pub fn accessor(&self) -> &impl StateAccessor {
        &self.accessor
    }
}
