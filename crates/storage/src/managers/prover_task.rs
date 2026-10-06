//! Database operations for stored prover task keys.
//!
//! [`crate::VersionedTaskStore`] adds the spec prefix and implements the service's
//! task-store interface. This manager handles the stored keys and database errors.

use std::sync::Arc;

use strata_db_types::errors::DbError;
use strata_db_types::prover_task::ProverTaskDatabase;
use strata_db_types::DbResult;
use strata_paas::{ProverError, ProverResult, TaskRecord, TaskRecordData, TaskStatus};
use tokio::runtime::Handle;

use crate::ops::prover_task::ProverTaskDbOps;

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct ProverTaskDbManager {
    ops: ProverTaskDbOps,
}

impl ProverTaskDbManager {
    pub fn new(handle: Handle, db: Arc<impl ProverTaskDatabase + 'static>) -> Self {
        let ops = ProverTaskDbOps::new(handle, db);
        Self { ops }
    }

    /// Counts keys with the given prefix and encoded length without collecting records.
    pub(crate) fn count_tasks_with_prefix(
        &self,
        prefix: [u8; 4],
        suffix_len: usize,
    ) -> ProverResult<usize> {
        self.ops
            .count_tasks_with_prefix_blocking(prefix, suffix_len)
            .map_err(db_err)
    }

    /// Lists due retry or blocked tasks in one namespace.
    pub(crate) fn list_retriable_with_prefix(
        &self,
        prefix: [u8; 4],
        suffix_len: usize,
        now_secs: u64,
    ) -> ProverResult<Vec<TaskRecord>> {
        let items = self
            .ops
            .list_retriable_with_prefix_blocking(prefix, suffix_len, now_secs)
            .map_err(db_err)?;
        Ok(items
            .into_iter()
            .map(|(key, data)| TaskRecord::from_parts(key, data))
            .collect())
    }

    /// Lists Pending or Proving tasks in one namespace.
    pub(crate) fn list_unfinished_with_prefix(
        &self,
        prefix: [u8; 4],
        suffix_len: usize,
    ) -> ProverResult<Vec<TaskRecord>> {
        let items = self
            .ops
            .list_unfinished_with_prefix_blocking(prefix, suffix_len)
            .map_err(db_err)?;
        Ok(items
            .into_iter()
            .map(|(key, data)| TaskRecord::from_parts(key, data))
            .collect())
    }

    /// Deletes a task record by key.
    pub fn delete_task(&self, key: &[u8]) -> DbResult<bool> {
        self.ops.delete_task_blocking(key.to_vec())
    }
}

fn db_err(e: DbError) -> ProverError {
    match e {
        DbError::EntryAlreadyExists => ProverError::TaskAlreadyExists(String::new()),
        other => ProverError::Storage(other.to_string()),
    }
}

impl ProverTaskDbManager {
    pub(crate) fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
        let stored = self.ops.get_task_blocking(key.to_vec()).map_err(db_err)?;
        Ok(stored.map(|data| TaskRecord::from_parts(key.to_vec(), data)))
    }

    pub(crate) fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        let (key, data) = (record.key().to_vec(), record.data().clone());
        self.ops
            .insert_task_blocking(key.clone(), data)
            .map_err(|e| match e {
                DbError::EntryAlreadyExists => ProverError::TaskAlreadyExists(format!("{:?}", key)),
                other => ProverError::Storage(other.to_string()),
            })
    }

    pub(crate) fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        self.modify(key, |d| d.set_status(status))
    }

    pub(crate) fn set_retry_after(&self, key: &[u8], when_secs: u64) -> ProverResult<()> {
        self.modify(key, |d| d.set_retry_after_secs(Some(when_secs)))
    }

    pub(crate) fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.modify(key, |d| d.set_metadata(Some(data)))
    }

    pub(crate) fn clear_metadata(&self, key: &[u8]) -> ProverResult<()> {
        self.modify(key, |d| d.set_metadata(None))
    }

    #[cfg(test)]
    pub(crate) fn count(&self) -> ProverResult<usize> {
        self.ops.count_tasks_blocking().map_err(db_err)
    }
}

impl ProverTaskDbManager {
    /// Read-modify-write helper; pure storage-level, not exposed publicly.
    fn modify<F>(&self, key: &[u8], f: F) -> ProverResult<()>
    where
        F: FnOnce(&mut TaskRecordData),
    {
        let mut data = self
            .ops
            .get_task_blocking(key.to_vec())
            .map_err(db_err)?
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?;
        f(&mut data);
        self.ops
            .put_task_blocking(key.to_vec(), data)
            .map_err(db_err)
    }
}
