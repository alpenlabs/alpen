//! Types relating to constructing the genesis chainstate.

use arbitrary::Arbitrary;

use crate::{bridge_state, exec_env, l1};

/// Genesis data we use to construct the genesis state.
#[derive(Clone, Debug, Arbitrary)]
pub struct GenesisStateData {
    l1_state: l1::L1ViewState,
    operator_table: bridge_state::OperatorTable,
    exec_state: exec_env::ExecEnvState,
}

impl GenesisStateData {
    pub fn new(
        l1_state: l1::L1ViewState,
        operator_table: bridge_state::OperatorTable,
        exec_state: exec_env::ExecEnvState,
    ) -> Self {
        Self {
            l1_state,
            operator_table,
            exec_state,
        }
    }

    pub fn l1_state(&self) -> &l1::L1ViewState {
        &self.l1_state
    }

    pub fn operator_table(&self) -> &bridge_state::OperatorTable {
        &self.operator_table
    }

    pub fn exec_state(&self) -> &exec_env::ExecEnvState {
        &self.exec_state
    }
}
