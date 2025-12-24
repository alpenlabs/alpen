//! Types relating to constructing the genesis chainstate.

use arbitrary::Arbitrary;
use strata_state::exec_env::ExecEnvState;

use crate::l1_view::L1ViewState;

/// Genesis data we use to construct the genesis state.
#[derive(Clone, Debug, Arbitrary)]
pub struct GenesisStateData {
    l1_state: L1ViewState,
    exec_state: ExecEnvState,
}

impl GenesisStateData {
    pub fn new(l1_state: L1ViewState, exec_state: ExecEnvState) -> Self {
        Self {
            l1_state,
            exec_state,
        }
    }

    pub fn l1_state(&self) -> &L1ViewState {
        &self.l1_state
    }

    pub fn exec_state(&self) -> &ExecEnvState {
        &self.exec_state
    }
}
