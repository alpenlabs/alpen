//! EVM execution-derived block output.

use alloy_consensus::proofs::calculate_receipt_root;
use alpen_reth_evm::accumulate_logs_bloom;
use reth_evm::execute::BlockExecutionOutput;
use reth_primitives::Receipt as EthereumReceipt;
use revm_primitives::alloy_primitives::{B256, Bloom};

use crate::types::EvmHeaderIntrinsics;

/// Execution commitments produced while executing an EVM block.
///
/// These values are not state writes. They are compared against the EVM block
/// header during verification.
#[derive(Clone, Debug)]
pub struct EvmBlockOutput {
    receipts_root: B256,
    logs_bloom: Bloom,
    gas_used: u64,
    blob_gas_used: Option<u64>,
    requests_hash: Option<B256>,
}

impl EvmBlockOutput {
    /// Creates execution commitments from the header shape and execution result.
    pub fn from_header_and_output(
        header_intrinsics: &EvmHeaderIntrinsics,
        execution_output: &BlockExecutionOutput<EthereumReceipt>,
    ) -> Self {
        Self {
            logs_bloom: accumulate_logs_bloom(&execution_output.result.receipts),
            receipts_root: calculate_receipt_root(&execution_output.result.receipts),
            gas_used: execution_output.result.gas_used,
            blob_gas_used: header_intrinsics
                .has_blob_gas_used()
                .then_some(execution_output.result.blob_gas_used),
            requests_hash: header_intrinsics
                .has_requests_hash()
                .then_some(execution_output.result.requests.requests_hash()),
        }
    }

    /// Creates execution commitments for a block.
    pub fn new(
        receipts_root: B256,
        logs_bloom: Bloom,
        gas_used: u64,
        blob_gas_used: Option<u64>,
        requests_hash: Option<B256>,
    ) -> Self {
        Self {
            receipts_root,
            logs_bloom,
            gas_used,
            blob_gas_used,
            requests_hash,
        }
    }

    /// Gets the receipts root produced by execution.
    pub fn receipts_root(&self) -> B256 {
        self.receipts_root
    }

    /// Gets the accumulated logs bloom.
    pub fn logs_bloom(&self) -> Bloom {
        self.logs_bloom
    }

    /// Gets the total gas used by execution.
    pub fn gas_used(&self) -> u64 {
        self.gas_used
    }

    /// Gets the blob gas used by execution.
    pub fn blob_gas_used(&self) -> Option<u64> {
        self.blob_gas_used
    }

    /// Gets the EIP-7685 requests hash produced by execution.
    pub fn requests_hash(&self) -> Option<B256> {
        self.requests_hash
    }
}
