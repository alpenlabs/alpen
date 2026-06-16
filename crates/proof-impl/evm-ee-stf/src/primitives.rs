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

/// Policy-relevant [`BridgeParams`] fields committed by the EVM EE STF proof public statement.
///
/// This statement must mirror every bridge parameter field that affects execution policy.
#[derive(
    Clone, Copy, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct BridgeParamsStatement {
    denomination: u64,
    max_withdrawal_amount: Option<u64>,
    max_withdrawal_descriptor_len: u32,
}

impl BridgeParamsStatement {
    pub fn from_bridge_params(bridge_params: BridgeParams) -> Self {
        Self {
            denomination: bridge_params.denomination(),
            max_withdrawal_amount: bridge_params.max_withdrawal_amount(),
            max_withdrawal_descriptor_len: bridge_params.max_withdrawal_descriptor_len(),
        }
    }

    pub fn denomination(&self) -> u64 {
        self.denomination
    }

    pub fn max_withdrawal_amount(&self) -> Option<u64> {
        self.max_withdrawal_amount
    }

    pub fn max_withdrawal_descriptor_len(&self) -> u32 {
        self.max_withdrawal_descriptor_len
    }
}

/// Public statement proven by the EVM EE STF proof.
#[derive(Clone, Debug, Eq, PartialEq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct EvmEeProofOutput {
    /// Bridge parameters used while executing the proved blocks.
    bridge_params: BridgeParamsStatement,
    /// Execution updates derived from the proved blocks.
    segments: Vec<ExecSegment>,
}

impl EvmEeProofOutput {
    pub fn new(bridge_params: BridgeParams, segments: Vec<ExecSegment>) -> Self {
        Self {
            bridge_params: BridgeParamsStatement::from_bridge_params(bridge_params),
            segments,
        }
    }

    /// Bridge parameters committed as part of the public proof statement.
    pub fn bridge_params(&self) -> &BridgeParamsStatement {
        &self.bridge_params
    }

    /// Execution segments committed as part of the public proof statement.
    pub fn segments(&self) -> &[ExecSegment] {
        &self.segments
    }
}

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
