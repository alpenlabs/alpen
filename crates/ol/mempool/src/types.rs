//! Core types for mempool operations.

use std::collections::HashMap;

/// Metadata stored with each OL transaction for ordering and management.
#[derive(Clone, Debug)]
pub struct MempoolTxMetadata {
    /// Slot when transaction was added to mempool (for FIFO ordering).
    pub entry_slot: u64,

    /// Unix timestamp when transaction was added (for metrics).
    pub entry_time: u64,

    /// Size of the transaction in bytes.
    pub size_bytes: usize,
    // TODO: Add fee field for priority ordering.
}

/// Statistics about the mempool state.
#[derive(Clone, Debug, Default)]
pub struct MempoolStats {
    /// Number of transactions currently in mempool.
    pub current_tx_count: usize,

    /// Total size in bytes of transactions currently in mempool.
    pub current_total_size: usize,

    /// Total number of transactions enqueued since startup.
    pub enqueued_tx_total: u64,

    /// Total number of transactions rejected since startup.
    pub rejected_tx_total: u64,

    /// Total number of transactions evicted since startup.
    pub evicted_tx_total: u64,

    /// Breakdown of rejections by reason.
    pub rejections_by_reason: HashMap<String, u64>,
}
