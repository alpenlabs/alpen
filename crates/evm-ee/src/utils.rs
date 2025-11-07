//! Utility functions for EVM block execution.
//!
//! This module contains utility functions used during block execution that don't
//! belong to any specific type.

use alloy_consensus::{Block as AlloyBlock, TxReceipt};
use alpen_reth_evm::collect_withdrawal_intents;
use reth_evm::execute::{BlockExecutionOutput, ExecutionOutcome};
use reth_primitives::{Receipt as EthereumReceipt, RecoveredBlock, TransactionSigned};
use reth_primitives_traits::Block;
use reth_trie::{HashedPostState, KeccakKeyHasher};
use revm_primitives::alloy_primitives::Bloom;
use strata_ee_acct_types::{EnvError, EnvResult, ExecPayload};

use crate::types::EvmBlock;

/// Builds an Alloy block from exec payload and recovers transaction senders.
///
/// This constructs an AlloyBlock from the header and body in the exec payload,
/// then recovers the sender addresses from transaction signatures.
pub(crate) fn build_and_recover_block(
    exec_payload: &ExecPayload<'_, EvmBlock>,
) -> EnvResult<RecoveredBlock<AlloyBlock<TransactionSigned>>> {
    let header = exec_payload.header_intrinsics().clone();
    let body = exec_payload.body().body().clone();

    // Build block using alloy_consensus types
    let alloy_block = AlloyBlock {
        header: header.clone(),
        body,
    };

    // Recover transaction senders from signatures
    alloy_block
        .try_into_recovered()
        .map_err(|_| EnvError::InvalidBlock)
}

/// Accumulates logs bloom from all receipts in the execution output.
pub(crate) fn accumulate_logs_bloom(receipts: &[EthereumReceipt]) -> Bloom {
    let mut logs_bloom = Bloom::default();
    receipts.iter().for_each(|r: &EthereumReceipt| {
        logs_bloom.accrue_bloom(&r.bloom());
    });
    logs_bloom
}

/// Collects withdrawal intents from executed transactions and their receipts.
pub(crate) fn collect_withdrawal_intents_from_execution(
    transactions: Vec<TransactionSigned>,
    receipts: &[EthereumReceipt],
) -> Vec<alpen_reth_primitives::WithdrawalIntent> {
    let executed_txns = transactions.iter();
    let receipt_refs = receipts.iter();
    let tx_receipt_pairs = executed_txns.zip(receipt_refs);
    collect_withdrawal_intents(tx_receipt_pairs).collect()
}

/// Converts execution output to HashedPostState for state updates.
pub(crate) fn compute_hashed_post_state(
    execution_output: &BlockExecutionOutput<EthereumReceipt>,
    block_number: u64,
) -> HashedPostState {
    let executor_outcome = ExecutionOutcome::new(
        execution_output.state.clone(),
        vec![execution_output.result.receipts.clone()],
        block_number,
        vec![execution_output.result.requests.clone()],
    );
    executor_outcome.hash_state_slow::<KeccakKeyHasher>()
}
