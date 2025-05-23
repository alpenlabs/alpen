use alloy_eips::eip4895::Withdrawal;
use alpen_reth_primitives::WithdrawalIntent;
use revm_primitives::alloy_primitives::FixedBytes;
use rsp_client_executor::io::EthClientExecutorInput;
use serde::{Deserialize, Serialize};
use strata_state::block::ExecSegment;

/// Public Parameters that proof asserts
pub type EvmEeProofOutput = Vec<ExecSegment>;

/// Input to the block execution
pub type EvmBlockStfInput = EthClientExecutorInput;

/// Public Parameters that proof asserts
pub type EvmEeProofInput = Vec<EthClientExecutorInput>;

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
