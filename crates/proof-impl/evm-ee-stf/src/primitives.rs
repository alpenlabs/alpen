use alloy_eips::eip4895::Withdrawal;
use alpen_reth_primitives::WithdrawalIntent;
use borsh::{BorshDeserialize, BorshSerialize};
use revm_primitives::alloy_primitives::FixedBytes;
use rsp_client_executor::io::EthClientExecutorInput;
use serde::{Deserialize, Serialize};
use strata_bridge_params::BridgeParams;
use strata_state::exec_update::ExecUpdate;

/// Information relating to how to update the execution layer.
///
/// Right now this just contains a single execution update since we only have a
/// single execution environment in our execution layer.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct ExecSegment {
    /// Update payload for the single execution environment.
    update: ExecUpdate,
}

impl ExecSegment {
    pub fn new(update: ExecUpdate) -> Self {
        Self { update }
    }

    /// The EE update payload.
    pub fn update(&self) -> &ExecUpdate {
        &self.update
    }
}

/// Public Parameters that proof asserts
pub type EvmEeProofOutput = Vec<ExecSegment>;

/// Input to the block execution
pub type EvmBlockStfInput = EthClientExecutorInput;

/// Public Parameters that proof asserts
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EvmEeProofInput {
    pub bridge_params: BridgeParams,
    pub block_inputs: Vec<EvmBlockStfInput>,
}

impl EvmEeProofInput {
    pub fn new(bridge_params: BridgeParams, block_inputs: Vec<EvmBlockStfInput>) -> Self {
        Self {
            bridge_params,
            block_inputs,
        }
    }
}

/// Result of the block execution
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvmBlockStfOutput {
    pub block_idx: u64,
    pub prev_blockhash: FixedBytes<32>,
    pub new_blockhash: FixedBytes<32>,
    pub new_state_root: FixedBytes<32>,
    pub txn_root: FixedBytes<32>,
    pub withdrawal_intents: Vec<WithdrawalIntent>,
    pub deposit_requests: Vec<Withdrawal>,
}
