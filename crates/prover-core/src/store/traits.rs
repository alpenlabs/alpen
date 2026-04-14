//! Task storage trait.

use std::time::SystemTime;

use crate::{error::ProverResult, task::TaskStatus};

/// A single task record in the store, keyed by opaque bytes (serialized `H::Task`).
#[derive(Debug, Clone)]
pub struct TaskRecord {
    key: Vec<u8>,
    status: TaskStatus,
    updated_at: SystemTime,
    retry_after: Option<SystemTime>,
    /// Opaque bytes for strategy-specific state (e.g. remote ProofId for crash recovery).
    metadata: Option<Vec<u8>>,
}

impl TaskRecord {
    pub fn new(key: Vec<u8>, status: TaskStatus) -> Self {
        Self {
            key,
            status,
            updated_at: SystemTime::now(),
            retry_after: None,
            metadata: None,
        }
    }

    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub fn status(&self) -> &TaskStatus {
        &self.status
    }

    pub fn retry_after(&self) -> Option<SystemTime> {
        self.retry_after
    }

    pub fn metadata(&self) -> Option<&[u8]> {
        self.metadata.as_deref()
    }

    pub fn update_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at = SystemTime::now();
    }

    pub fn set_retry_after(&mut self, when: Option<SystemTime>) {
        self.retry_after = when;
        self.updated_at = SystemTime::now();
    }

    pub fn set_metadata(&mut self, data: Option<Vec<u8>>) {
        self.metadata = data;
        self.updated_at = SystemTime::now();
    }
}

/// Persistence for task records. Keyed by opaque bytes, no generics.
///
/// All methods return [`ProverResult`] so backends can surface IO/decode
/// errors to callers instead of silently discarding them.
pub trait TaskStore: Send + Sync + 'static {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>>;
    fn insert(&self, record: TaskRecord) -> ProverResult<()>;
    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()>;
    fn set_retry_after(&self, key: &[u8], when: SystemTime) -> ProverResult<()>;
    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()>;
    fn list_retriable(&self, now: SystemTime) -> ProverResult<Vec<TaskRecord>>;
    /// Every record that was submitted but hasn't reached a terminal state —
    /// Pending, Queued, or Proving. Used by startup recovery to re-spawn
    /// work that was interrupted by a crash before it completed.
    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>>;
    fn count(&self) -> ProverResult<usize>;
}
