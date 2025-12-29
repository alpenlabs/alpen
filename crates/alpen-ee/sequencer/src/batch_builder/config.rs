//! Configuration for the batch builder task.

/// Configuration for the batch builder task.
#[derive(Debug, Clone)]
pub struct BatchBuilderConfig {
    /// Maximum blocks per batch (for block count policy).
    pub max_blocks_per_batch: u64,
    /// Backoff duration (ms) when block data is not yet available.
    pub data_poll_interval_ms: u64,
    /// Backoff duration (ms) on errors.
    pub error_backoff_ms: u64,
}

impl Default for BatchBuilderConfig {
    fn default() -> Self {
        Self {
            max_blocks_per_batch: 100,
            data_poll_interval_ms: 100,
            error_backoff_ms: 1000,
        }
    }
}

impl BatchBuilderConfig {
    /// Create a new configuration with the specified max blocks per batch.
    pub fn with_max_blocks(max_blocks_per_batch: u64) -> Self {
        Self {
            max_blocks_per_batch,
            ..Default::default()
        }
    }
}
