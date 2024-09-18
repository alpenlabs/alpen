//! Core logic of the Bitcoin Blockspace proof that will be proven

use bitcoin::{block::Header, Block, ScriptBuf};
use serde::{Deserialize, Serialize};

use crate::{
    block::check_merkle_root,
    filter::{extract_relevant_transactions, DepositRequestData, ForcedInclusion, StateUpdate},
};

#[derive(Debug, Serialize, Deserialize)]
pub struct BlockspaceProofInput {
    pub block: Block,
    pub scan_config: ScanRuleConfig,
    // TODO: add hintings and other necessary params
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanRuleConfig {
    pub bridge_scriptbufs: Vec<ScriptBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BlockspaceProofOutput {
    pub header: Header,
    pub deposits: Vec<DepositRequestData>,
    pub forced_inclusions: Vec<ForcedInclusion>,
    pub state_updates: Vec<StateUpdate>,
}

pub fn process_blockspace_proof(input: &BlockspaceProofInput) -> BlockspaceProofOutput {
    let BlockspaceProofInput { block, scan_config } = input;
    assert!(check_merkle_root(block));
    // assert!(check_witness_commitment(block));

    let (deposits, forced_inclusions, state_updates) =
        extract_relevant_transactions(block, scan_config);

    BlockspaceProofOutput {
        header: block.header,
        deposits,
        forced_inclusions,
        state_updates,
    }
}
