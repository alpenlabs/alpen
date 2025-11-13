//! Core types for mempool operations.

use std::collections::HashMap;

/// Default maximum number of transactions in mempool.
pub const DEFAULT_MAX_TX_COUNT: usize = 10_000;

/// Default maximum size of a single transaction (1 MB).
pub const DEFAULT_MAX_TX_SIZE: usize = 1024 * 1024;

/// Configuration for mempool behavior.
#[derive(Debug, Clone)]
pub struct MempoolConfig {
    /// Maximum number of transactions in mempool.
    pub max_tx_count: usize,

    /// Maximum size of a single transaction in bytes.
    pub max_tx_size: usize,
}

impl Default for MempoolConfig {
    fn default() -> Self {
        Self {
            max_tx_count: DEFAULT_MAX_TX_COUNT,
            max_tx_size: DEFAULT_MAX_TX_SIZE,
        }
    }
}

impl MempoolConfig {
    /// Maximum total size in bytes (derived from count and individual size limits).
    pub fn max_total_size(&self) -> usize {
        self.max_tx_count * self.max_tx_size
    }
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
