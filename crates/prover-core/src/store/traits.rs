//! Task storage trait.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use borsh::{BorshDeserialize, BorshSerialize};

use crate::{error::ProverResult, task::TaskStatus};

/// Seconds since UNIX epoch, monotonic wall-clock.
///
/// Using a bare `u64` (instead of [`SystemTime`]) keeps the record shape
/// borsh-stable so storage backends can persist it without a conversion
/// layer. Sub-second precision isn't needed anywhere in the prover: retry
/// scheduling and status timestamps are both on second granularity.
pub type SecsSinceEpoch = u64;

/// Current wall-clock seconds since UNIX epoch.
pub fn now_secs() -> SecsSinceEpoch {
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
/// All time fields are [`SecsSinceEpoch`] so the record is directly
/// borsh-serializable — persistent backends store this type as-is, no
/// on-disk shadow type, no conversion.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct TaskRecordData {
    status: TaskStatus,
    updated_at_secs: SecsSinceEpoch,
    retry_after_secs: Option<SecsSinceEpoch>,
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

    pub fn updated_at_secs(&self) -> SecsSinceEpoch {
        self.updated_at_secs
    }

    pub fn retry_after_secs(&self) -> Option<SecsSinceEpoch> {
        self.retry_after_secs
    }

    pub fn metadata(&self) -> Option<&[u8]> {
        self.metadata.as_deref()
    }

    pub fn set_status(&mut self, status: TaskStatus) {
        self.status = status;
        self.updated_at_secs = now_secs();
    }

    pub fn set_retry_after_secs(&mut self, when: Option<SecsSinceEpoch>) {
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

    pub fn retry_after_secs(&self) -> Option<SecsSinceEpoch> {
        self.data.retry_after_secs()
    }

    pub fn metadata(&self) -> Option<&[u8]> {
        self.data.metadata()
    }
}

/// Persistence for task records. Keyed by opaque bytes, no generics.
///
/// All methods return [`ProverResult`] so backends can surface IO/decode
/// errors to callers instead of silently discarding them. Times are
/// [`SecsSinceEpoch`] to match the record layout.
pub trait TaskStore: Send + Sync + 'static {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>>;
    fn insert(&self, record: TaskRecord) -> ProverResult<()>;
    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()>;
    fn set_retry_after(&self, key: &[u8], when_secs: SecsSinceEpoch) -> ProverResult<()>;
    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()>;
    fn list_retriable(&self, now_secs: SecsSinceEpoch) -> ProverResult<Vec<TaskRecord>>;
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
    fn set_retry_after(&self, key: &[u8], when_secs: SecsSinceEpoch) -> ProverResult<()> {
        (**self).set_retry_after(key, when_secs)
    }
    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        (**self).set_metadata(key, data)
    }
    fn list_retriable(&self, now_secs: SecsSinceEpoch) -> ProverResult<Vec<TaskRecord>> {
        (**self).list_retriable(now_secs)
    }
    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
        (**self).list_unfinished()
    }
    fn count(&self) -> ProverResult<usize> {
        (**self).count()
    }
}
