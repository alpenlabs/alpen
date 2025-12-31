//! Centralized metrics collection for node sync performance profiling.
//!
//! This module provides Prometheus metrics for tracking:
//! - Block processing timing (validation, execution, state transition, DB writes)
//! - RPC call timing and payload sizes
//! - Database operation timing and sizes
//! - CPU usage indicators
//!
//! Metrics are exposed via a Prometheus HTTP endpoint for scraping.

use lazy_static::lazy_static;
use prometheus::{
    register_histogram_vec, register_int_counter_vec, register_int_gauge_vec, HistogramOpts,
    HistogramVec, IntCounterVec, IntGaugeVec, Registry,
};
use std::time::Instant;

lazy_static! {
    /// Global registry for all metrics
    pub static ref REGISTRY: Registry = Registry::new();

    // ==================== Block Processing Metrics ====================

    /// Histogram tracking block processing duration by stage
    /// Labels: stage=[validation|execution|state_transition|db_write|total]
    pub static ref BLOCK_PROCESSING_DURATION: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "strata_block_processing_duration_seconds",
            "Time spent processing blocks by stage"
        )
        .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0]),
        &["stage"]
    )
    .unwrap();

    /// Counter for total blocks processed
    pub static ref BLOCKS_PROCESSED_TOTAL: IntCounterVec = register_int_counter_vec!(
        "strata_blocks_processed_total",
        "Total number of blocks processed",
        &["status"] // status=[success|failed]
    )
    .unwrap();

    /// Gauge tracking current sync tip block height
    pub static ref SYNC_TIP_BLOCK_HEIGHT: IntGaugeVec = register_int_gauge_vec!(
        "strata_sync_tip_block_height",
        "Current sync tip block height",
        &["chain"] // chain=[l1|l2]
    )
    .unwrap();

    /// Gauge tracking sync lag in blocks
    pub static ref SYNC_LAG_BLOCKS: IntGaugeVec = register_int_gauge_vec!(
        "strata_sync_lag_blocks",
        "Number of blocks behind the network tip",
        &["chain"] // chain=[l1|l2]
    )
    .unwrap();

    // ==================== Database Metrics ====================

    /// Histogram tracking database write operation duration
    /// Labels: operation=[put_block|put_chainstate|put_client_state|batch_write]
    pub static ref DB_WRITE_DURATION: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "strata_db_write_duration_seconds",
            "Time spent on database write operations"
        )
        .buckets(vec![0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0]),
        &["operation"]
    )
    .unwrap();

    /// Histogram tracking database write payload size in bytes
    /// Labels: operation=[put_block|put_chainstate|put_client_state|batch_write]
    pub static ref DB_WRITE_BYTES: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "strata_db_write_bytes",
            "Size of data written to database"
        )
        .buckets(vec![100.0, 1_000.0, 10_000.0, 100_000.0, 1_000_000.0, 10_000_000.0]),
        &["operation"]
    )
    .unwrap();

    /// Counter for total database operations
    pub static ref DB_OPERATIONS_TOTAL: IntCounterVec = register_int_counter_vec!(
        "strata_db_operations_total",
        "Total number of database operations",
        &["operation", "status"] // status=[success|failed]
    )
    .unwrap();

    // ==================== RPC/Network Metrics ====================

    /// Histogram tracking RPC call duration
    /// Labels: endpoint=[submit_payload|update_safe_block|get_blocks_range|...]
    pub static ref RPC_CALL_DURATION: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "strata_rpc_call_duration_seconds",
            "Time spent on RPC calls"
        )
        .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 30.0]),
        &["endpoint", "target"] // target=[execution_engine|l2_sync_peer]
    )
    .unwrap();

    /// Histogram tracking RPC request/response payload size
    /// Labels: endpoint, direction=[request|response]
    pub static ref RPC_PAYLOAD_BYTES: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "strata_rpc_payload_bytes",
            "Size of RPC request and response payloads"
        )
        .buckets(vec![100.0, 1_000.0, 10_000.0, 100_000.0, 1_000_000.0, 10_000_000.0]),
        &["endpoint", "direction", "target"]
    )
    .unwrap();

    /// Counter for total RPC calls
    pub static ref RPC_CALLS_TOTAL: IntCounterVec = register_int_counter_vec!(
        "strata_rpc_calls_total",
        "Total number of RPC calls",
        &["endpoint", "target", "status"] // status=[success|failed|timeout]
    )
    .unwrap();

    // ==================== Consensus State Machine Metrics ====================

    /// Histogram tracking CSM event processing duration
    pub static ref CSM_EVENT_DURATION: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "strata_csm_event_duration_seconds",
            "Time spent processing CSM events"
        )
        .buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0]),
        &["event_type"] // event_type=[l1_block|l2_genesis|checkpoint|...]
    )
    .unwrap();

    /// Counter for CSM retry attempts
    pub static ref CSM_RETRY_TOTAL: IntCounterVec = register_int_counter_vec!(
        "strata_csm_retry_total",
        "Total number of CSM event retry attempts",
        &["event_type"]
    )
    .unwrap();

    // ==================== L2 Sync Metrics ====================

    /// Histogram tracking L2 block fetch duration
    pub static ref L2_BLOCK_FETCH_DURATION: HistogramVec = register_histogram_vec!(
        HistogramOpts::new(
            "strata_l2_block_fetch_duration_seconds",
            "Time spent fetching L2 blocks from peers"
        )
        .buckets(vec![0.01, 0.05, 0.1, 0.5, 1.0, 2.0, 5.0, 10.0]),
        &["peer"]
    )
    .unwrap();

    /// Counter for total blocks fetched from L2 sync
    pub static ref L2_BLOCKS_FETCHED_TOTAL: IntCounterVec = register_int_counter_vec!(
        "strata_l2_blocks_fetched_total",
        "Total number of blocks fetched from L2 sync",
        &["peer", "status"] // status=[success|failed|missing_parent]
    )
    .unwrap();
}

/// Helper to register all metrics with the global registry
pub fn register_metrics() -> Result<(), prometheus::Error> {
    REGISTRY.register(Box::new(BLOCK_PROCESSING_DURATION.clone()))?;
    REGISTRY.register(Box::new(BLOCKS_PROCESSED_TOTAL.clone()))?;
    REGISTRY.register(Box::new(SYNC_TIP_BLOCK_HEIGHT.clone()))?;
    REGISTRY.register(Box::new(SYNC_LAG_BLOCKS.clone()))?;
    REGISTRY.register(Box::new(DB_WRITE_DURATION.clone()))?;
    REGISTRY.register(Box::new(DB_WRITE_BYTES.clone()))?;
    REGISTRY.register(Box::new(DB_OPERATIONS_TOTAL.clone()))?;
    REGISTRY.register(Box::new(RPC_CALL_DURATION.clone()))?;
    REGISTRY.register(Box::new(RPC_PAYLOAD_BYTES.clone()))?;
    REGISTRY.register(Box::new(RPC_CALLS_TOTAL.clone()))?;
    REGISTRY.register(Box::new(CSM_EVENT_DURATION.clone()))?;
    REGISTRY.register(Box::new(CSM_RETRY_TOTAL.clone()))?;
    REGISTRY.register(Box::new(L2_BLOCK_FETCH_DURATION.clone()))?;
    REGISTRY.register(Box::new(L2_BLOCKS_FETCHED_TOTAL.clone()))?;
    Ok(())
}

// ==================== Timing Helpers ====================

/// RAII guard for automatic timing of operations
pub struct TimingGuard {
    start: Instant,
    histogram: HistogramVec,
    labels: Vec<String>,
}

impl TimingGuard {
    pub fn new(histogram: HistogramVec, labels: Vec<String>) -> Self {
        Self {
            start: Instant::now(),
            histogram,
            labels,
        }
    }
}

impl Drop for TimingGuard {
    fn drop(&mut self) {
        let duration = self.start.elapsed().as_secs_f64();
        let label_refs: Vec<&str> = self.labels.iter().map(|s| s.as_str()).collect();
        self.histogram
            .with_label_values(&label_refs)
            .observe(duration);
    }
}

/// Helper macro for timing a block of code
#[macro_export]
macro_rules! time_block {
    ($histogram:expr, $labels:expr, $block:expr) => {{
        let _guard = $crate::metrics::TimingGuard::new($histogram.clone(), $labels);
        $block
    }};
}

/// Helper macro for timing async operations
#[macro_export]
macro_rules! time_async {
    ($histogram:expr, $labels:expr, $fut:expr) => {{
        let start = std::time::Instant::now();
        let result = $fut.await;
        let duration = start.elapsed().as_secs_f64();
        let label_refs: Vec<&str> = $labels.iter().map(|s| s.as_str()).collect();
        $histogram.with_label_values(&label_refs).observe(duration);
        result
    }};
}
