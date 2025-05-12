use strata_state::chain_state::Chainstate;

use crate::context::StateProvider;

/// Container that tracks writes on top of a database handle for the state we're
/// building on top of.
pub struct State<P: StateProvider> {
    provider: P,

    new_chainstate: Chainstate,
}

impl<P: StateProvider> State<P> {
    /// Constructs a new instance wrapping a previous state.
    pub fn new(provider: P, new_chainstate: Chainstate) -> Self {
        Self {
            provider,
            new_chainstate,
        }
    }

    pub fn cur_chainstate(&self) -> &Chainstate {
        &self.new_chainstate
    }
}
