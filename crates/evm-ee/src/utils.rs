//! Utility functions for EVM block execution.
//!
//! This module contains utility functions used during block execution that don't
//! belong to any specific type.

use alloy_consensus::Block as AlloyBlock;
use alpen_reth_evm::withdrawal_intents;
use reth_evm::execute::BlockExecutionOutput;
use reth_primitives::{Receipt as EthereumReceipt, RecoveredBlock, TransactionSigned};
use reth_primitives_traits::Block;
use reth_trie::{HashedPostState, KeccakKeyHasher};
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
    let alloy_block = AlloyBlock { header, body };

    // Recover transaction senders from signatures
    alloy_block
        .try_into_recovered()
        .map_err(|_| EnvError::InvalidBlock)
}

/// Collects withdrawal intents from executed transactions and their receipts.
pub(crate) fn collect_withdrawal_intents_from_execution(
    transactions: Vec<TransactionSigned>,
    receipts: &[EthereumReceipt],
) -> Vec<alpen_reth_primitives::WithdrawalIntent> {
    withdrawal_intents(&transactions, receipts).collect()
}

/// Converts execution output to HashedPostState for state updates.
pub(crate) fn compute_hashed_post_state(
    execution_output: BlockExecutionOutput<EthereumReceipt>,
    _block_number: u64,
) -> HashedPostState {
    HashedPostState::from_bundle_state::<KeccakKeyHasher>(&execution_output.state.state)
}
