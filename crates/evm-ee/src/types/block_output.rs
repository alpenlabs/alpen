//! EVM execution-derived block output.

use revm_primitives::alloy_primitives::{B256, Bloom};

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
