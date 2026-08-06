//! Replay output summary types.

use alpen_ee_da_types::{DaBlob, EvmHeaderSummary};
use rsp_mpt::EthereumState;
use serde::Serialize;
use strata_identifiers::Buf32;

/// Range of EVM execution blocks applied during replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppliedExecBlockRange {
    first_update_seq_no: u64,
    last_update_seq_no: u64,
    first_block_num: u64,
    last_block_num: u64,
    count: usize,
}

impl AppliedExecBlockRange {
    pub(crate) fn new(first_block_num: u64, first: &DaBlob, last: &DaBlob, count: usize) -> Self {
        Self {
            first_update_seq_no: first.update_seq_no,
            last_update_seq_no: last.update_seq_no,
            first_block_num,
            last_block_num: last.evm_header.block_num,
            count,
        }
    }

    /// Returns the first applied EE DA update sequence number.
    pub fn first_update_seq_no(&self) -> u64 {
        self.first_update_seq_no
    }

    /// Returns the last applied EE DA update sequence number.
    pub fn last_update_seq_no(&self) -> u64 {
        self.last_update_seq_no
    }

    /// Returns the first applied EVM block number.
    pub fn first_block_num(&self) -> u64 {
        self.first_block_num
    }

    /// Returns the last applied EVM block number.
    pub fn last_block_num(&self) -> u64 {
        self.last_block_num
    }

    /// Returns the number of DA blobs applied.
    pub fn count(&self) -> usize {
        self.count
    }
}

/// Replay-stage output summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplaySummary {
    applied: Option<AppliedExecBlockRange>,
    #[serde(skip)]
    applied_evm_headers: Vec<EvmHeaderSummary>,
    #[serde(skip)]
    applied_state_roots: Vec<Buf32>,
    final_state_root: Buf32,
    #[serde(skip)]
    final_ethereum_state: EthereumState,
}

impl ReplaySummary {
    pub(crate) fn new(
        applied: Option<AppliedExecBlockRange>,
        applied_evm_headers: Vec<EvmHeaderSummary>,
        applied_state_roots: Vec<Buf32>,
        final_state_root: Buf32,
        final_ethereum_state: EthereumState,
    ) -> Self {
        Self {
            applied,
            applied_evm_headers,
            applied_state_roots,
            final_state_root,
            final_ethereum_state,
        }
    }

    /// Returns the applied EVM block range, if any DA blobs were replayed.
    pub fn applied(&self) -> Option<&AppliedExecBlockRange> {
        self.applied.as_ref()
    }

    /// Returns the EVM header summaries for each applied DA blob.
    pub fn applied_evm_headers(&self) -> &[EvmHeaderSummary] {
        &self.applied_evm_headers
    }

    /// Returns the post-blob execution state root for each applied DA blob.
    pub fn applied_state_roots(&self) -> &[Buf32] {
        &self.applied_state_roots
    }

    /// Returns the reconstructed final state root.
    pub fn final_state_root(&self) -> Buf32 {
        self.final_state_root
    }

    /// Returns the final Ethereum state after replay.
    pub fn final_ethereum_state(&self) -> &EthereumState {
        &self.final_ethereum_state
    }
}
