//! This crate implements the aggregation of consecutive L1 blocks to form a single proof

use alpen_express_state::{
    batch::BatchCheckpoint,
    l1::{get_btc_params, HeaderVerificationState, HeaderVerificationStateSnapshot},
    tx::DepositInfo,
};
use borsh::{BorshDeserialize, BorshSerialize};
use express_proofimpl_btc_blockspace::logic::BlockspaceProofOutput;

#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct L1BatchProofInput {
    pub batch: Vec<BlockspaceProofOutput>,
    pub state: HeaderVerificationState,
}

#[derive(Debug, BorshSerialize, BorshDeserialize)]
pub struct L1BatchProofOutput {
    pub deposits: Vec<DepositInfo>,
    pub state_update: Option<BatchCheckpoint>,
    pub initial_snapshot: HeaderVerificationStateSnapshot,
    pub final_snapshot: HeaderVerificationStateSnapshot,
}

pub fn process_batch_proof(input: L1BatchProofInput) -> L1BatchProofOutput {
    let mut state = input.state;
    let initial_snapshot = state.compute_snapshot();
    let params = get_btc_params();

    let mut deposits = Vec::new();
    let mut state_update = None;
    for blockspace in input.batch {
        let header = bitcoin::consensus::deserialize(&blockspace.header_raw).unwrap();
        state.check_and_update_full(&header, &params);
        deposits.extend(blockspace.deposits);
        state_update = state_update.or(blockspace.state_update);
    }
    let final_snapshot = state.compute_snapshot();

    L1BatchProofOutput {
        deposits,
        state_update,
        initial_snapshot,
        final_snapshot,
    }
}
