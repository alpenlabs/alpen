//! Task storage trait.

use std::{sync::Arc, time::SystemTime};

use crate::{error::ProverResult, task::TaskStatus};

/// The mutable state associated with a stored task, separate from its key.
///
/// Splitting the key bytes from the value fields makes the dataflow
/// explicit: storage backends store `TaskRecordData` against a `Vec<u8>`
/// key, and [`TaskRecord`] is just the key-value pair surfaced to callers.
#[derive(Debug, Clone)]
pub struct TaskRecordData {
    status: TaskStatus,
    updated_at: SystemTime,
    retry_after: Option<SystemTime>,
    /// Opaque bytes for strategy-specific state (e.g. remote ProofId for crash recovery).
    metadata: Option<Vec<u8>>,
}

impl TaskRecordData {
    pub fn new(status: TaskStatus) -> Self {
        Self {
            status,
            updated_at: SystemTime::now(),
            retry_after: None,
            metadata: None,
        }
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

    pub fn set_status(&mut self, status: TaskStatus) {
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

    pub fn retry_after(&self) -> Option<SystemTime> {
        self.data.retry_after()
    }

    pub fn metadata(&self) -> Option<&[u8]> {
        self.data.metadata()
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

/// Pass an `Arc<impl TaskStore>` straight into the builder: the wrapping Arc
/// forwards every call to the inner store. Useful when the store is a
/// shared storage manager held elsewhere in the application.
impl<T: TaskStore + ?Sized> TaskStore for Arc<T> {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
        (**self).get(key)
    }
    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        (**self).insert(record)
    }
    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        (**self).update_status(key, status)
    }
    fn set_retry_after(&self, key: &[u8], when: SystemTime) -> ProverResult<()> {
        (**self).set_retry_after(key, when)
    }
    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        (**self).set_metadata(key, data)
    }
    fn list_retriable(&self, now: SystemTime) -> ProverResult<Vec<TaskRecord>> {
        (**self).list_retriable(now)
    }
    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
        (**self).list_unfinished()
    }
    fn count(&self) -> ProverResult<usize> {
        (**self).count()
    }
}
