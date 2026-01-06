//! Types relating to constructing the genesis chainstate.

use arbitrary::Arbitrary;
use strata_identifiers::{create_evm_extra_payload, Buf32, L1BlockCommitment};
use strata_state::{exec_env::ExecEnvState, exec_update::UpdateInput};

use crate::{l1_view::L1ViewState, Chainstate};

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

/// Compute the genesis OL chainstate root from the rollup parameters.
///
/// This deterministically derives the genesis state root from the rollup
/// configuration, enabling the checkpoint subprotocol to validate the
/// first checkpoint's pre-state root without requiring it as a separate input.
///
/// # Arguments
///
/// * `genesis_l1_blk` - The genesis L1 block commitment from rollup params.
/// * `evm_genesis_block_hash` - The EVM genesis block hash from rollup params.
/// * `evm_genesis_block_state_root` - The EVM genesis state root from rollup params.
///
/// # Returns
///
/// The computed OL chainstate root at genesis (slot 0).
pub fn compute_genesis_ol_state_root(
    genesis_l1_blk: L1BlockCommitment,
    evm_genesis_block_hash: Buf32,
    evm_genesis_block_state_root: Buf32,
) -> Buf32 {
    // Create the genesis UpdateInput with EVM extra payload.
    // This matches the logic in strata-consensus-logic::genesis::make_genesis_block.
    let extra_payload = create_evm_extra_payload(evm_genesis_block_hash);
    let genesis_update_input = UpdateInput::new(0, vec![], Buf32::zero(), extra_payload);

    // Create the execution environment state from the genesis input.
    let exec_state =
        ExecEnvState::from_base_input(genesis_update_input, evm_genesis_block_state_root);

    // Create the L1 view state at genesis.
    let l1_state = L1ViewState::new_at_genesis(genesis_l1_blk);

    // Construct genesis state data and compute the chainstate root.
    let genesis_data = GenesisStateData::new(l1_state, exec_state);
    Chainstate::from_genesis(&genesis_data).compute_state_root()
}
