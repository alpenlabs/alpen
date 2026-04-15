//! Task lifecycle types: status, result, stored record, and the
//! seconds-since-epoch time helpers that go with them.

use std::time::{SystemTime, UNIX_EPOCH};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

// ============================================================================
// TaskStatus / TaskResult
// ============================================================================

/// Status of a proof task in the lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, BorshSerialize, BorshDeserialize)]
pub enum TaskStatus {
    /// Task registered but not yet picked up for proving.
    Pending,
    /// Actively being proved.
    Proving,
    /// Proof completed successfully, receipt available.
    Completed,
    /// Temporary failure; will be retried after backoff.
    TransientFailure { retry_count: u32, error: String },
    /// Unrecoverable failure; task will not be retried.
    PermanentFailure { error: String },
}

impl TaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::PermanentFailure { .. })
    }

    pub fn is_retriable(&self) -> bool {
        matches!(self, Self::TransientFailure { .. })
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(self, Self::Proving)
    }

    /// True for any status that should be re-spawned on startup recovery:
    /// tasks that were submitted but never finished (Pending / Proving).
    /// Transient failures are handled separately by the retry scanner via
    /// [`Self::is_retriable`].
    pub fn is_unfinished(&self) -> bool {
        matches!(self, Self::Pending | Self::Proving)
    }
}

/// Outcome of a completed (or failed) task. Returned by `execute` and `wait_for_tasks`.
#[derive(Debug, Clone)]
pub enum TaskResult<T> {
    Completed { task: T },
    Failed { task: T, error: String },
}

impl<T> TaskResult<T> {
    pub fn completed(task: T) -> Self {
        Self::Completed { task }
    }

    pub fn failed(task: T, error: impl Into<String>) -> Self {
        Self::Failed {
            task,
            error: error.into(),
        }
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed { .. })
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn task(&self) -> &T {
        match self {
            Self::Completed { task } | Self::Failed { task, .. } => task,
        }
    }
}

// ============================================================================
// Stored record shape + time helpers
// ============================================================================

/// Current wall-clock seconds since UNIX epoch.
///
/// Internal helper — timestamps in task records are plain `u64` seconds
/// since epoch so the record is borsh-stable.
pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// The mutable state associated with a stored task, separate from its key.
///
/// Splitting the key bytes from the value fields makes the dataflow
/// explicit: storage backends store `TaskRecordData` against a `Vec<u8>`
/// key, and [`TaskRecord`] is just the key-value pair surfaced to callers.
///
/// All time fields are `u64` seconds since UNIX epoch so the record is
/// directly borsh-serializable — persistent backends store this type as-is,
/// no on-disk shadow type, no conversion. Sub-second precision isn't
/// needed anywhere in the prover.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct TaskRecordData {
    status: TaskStatus,
    updated_at_secs: u64,
    retry_after_secs: Option<u64>,
    /// Opaque bytes for strategy-specific state (e.g. remote ProofId for crash recovery).
    metadata: Option<Vec<u8>>,
}

impl TaskRecordData {
    pub fn new(status: TaskStatus) -> Self {
        Self {
            status,
            updated_at_secs: now_secs(),
            retry_after_secs: None,
            metadata: None,
        }
    }

    pub fn status(&self) -> &TaskStatus {
        &self.status
    }

    pub fn updated_at_secs(&self) -> u64 {
        self.updated_at_secs
    }

    pub fn retry_after_secs(&self) -> Option<u64> {
        self.retry_after_secs
    }

    pub fn metadata(&self) -> Option<&[u8]> {
        self.metadata.as_deref()
    }

    pub fn set_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at_secs = now_secs();
    }

    pub fn set_retry_after_secs(&mut self, when: Option<u64>) {
        self.retry_after_secs = when;
        self.updated_at_secs = now_secs();
    }

    pub fn set_metadata(&mut self, data: Option<Vec<u8>>) {
        self.metadata = data;
        self.updated_at_secs = now_secs();
    }
}

/// A stored task: the opaque byte key plus its associated [`TaskRecordData`].
#[derive(Debug, Clone)]
pub struct TaskRecord {
    key: Vec<u8>,
    data: TaskRecordData,
}

impl TaskRecord {
    pub fn new(key: Vec<u8>, status: TaskStatus) -> Self {
        Self {
            key,
            data: TaskRecordData::new(status),
        }
    }

    pub fn from_parts(key: Vec<u8>, data: TaskRecordData) -> Self {
        Self { key, data }
    }

    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub fn data(&self) -> &TaskRecordData {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut TaskRecordData {
        &mut self.data
    }

    pub fn status(&self) -> &TaskStatus {
        self.data.status()
    }

    pub fn retry_after_secs(&self) -> Option<u64> {
        self.data.retry_after_secs()
    }

    pub fn metadata(&self) -> Option<&[u8]> {
        self.data.metadata()
    }
}
