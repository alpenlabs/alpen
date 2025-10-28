//! Status types for PaaS service

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Current status of the PaaS service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaaSStatus {
    /// Number of active (proving) tasks
    pub active_tasks: usize,

    /// Number of queued tasks waiting for workers
    pub queued_tasks: usize,

    /// Total completed tasks (since service start)
    pub completed_tasks: usize,

    /// Total failed tasks (since service start)
    pub failed_tasks: usize,

    /// Worker pool utilization (0.0 - 1.0)
    pub worker_utilization: f32,
}

/// Detailed metrics report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaaSReport {
    /// Total proofs requested
    pub total_proofs: u64,

    /// Successfully completed proofs
    pub completed_proofs: u64,

    /// Failed proofs
    pub failed_proofs: u64,

    /// Average proof generation duration (milliseconds)
    pub average_duration_ms: u64,

    /// Worker statistics per backend
    pub worker_stats: WorkerStats,
}

/// Worker statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStats {
    /// Total number of workers
    pub total_workers: usize,

    /// Currently busy workers
    pub busy_workers: usize,

    /// Available workers
    pub available_workers: usize,
}

/// Status of a proof task
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task created, waiting to be queued
    Pending,

    /// Task in worker queue, waiting for available worker
    Queued,

    /// Proof generation in progress
    Proving {
        /// Generation progress (0.0 - 1.0)
        progress: f32,
        /// When proving started
        started_at: DateTime<Utc>,
    },

    /// Proof successfully generated
    Completed {
        /// When proof completed
        completed_at: DateTime<Utc>,
        /// Duration in milliseconds
        duration_ms: u64,
    },

    /// Permanent failure (no more retries)
    Failed {
        /// When task failed
        failed_at: DateTime<Utc>,
        /// Error message
        error: String,
        /// Number of retries attempted
        retry_count: u32,
    },

    /// Transient failure, will be retried
    TransientFailure {
        /// When failure occurred
        failed_at: DateTime<Utc>,
        /// Error message
        error: String,
        /// Current retry count
        retry_count: u32,
        /// When next retry will be attempted
        next_retry_at: DateTime<Utc>,
    },

    /// Task cancelled by user
    Cancelled {
        /// When task was cancelled
        cancelled_at: DateTime<Utc>,
    },
}

impl TaskStatus {
    /// Check if task is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed { .. } | TaskStatus::Failed { .. } | TaskStatus::Cancelled { .. }
        )
    }

    /// Check if task is active (queued or proving)
    pub fn is_active(&self) -> bool {
        matches!(self, TaskStatus::Queued | TaskStatus::Proving { .. })
    }
}
