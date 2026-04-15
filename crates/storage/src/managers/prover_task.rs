//! High-level manager for the prover task store.
//!
//! Wraps [`ProverTaskDatabase`] with a threadpool + instrumentation, and
//! implements [`strata_paas::TaskStore`] directly so the integrated prover
//! service can consume the manager as its persistent task store without
//! any extra adapter. The DB trait stores [`TaskRecordData`] verbatim, so
//! this layer is a thin translation between the `(key, data)` split used
//! by `TaskStore` and the `(key, record)` tuples used by the DB trait.

use std::sync::Arc;

use strata_db_types::{errors::DbError, traits::ProverTaskDatabase};
use strata_paas::{
    ProverError, ProverResult, SecsSinceEpoch, TaskRecord, TaskRecordData, TaskStatus, TaskStore,
};
use threadpool::ThreadPool;

use crate::ops::prover_task::{Context, ProverTaskDbOps};

#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug implementation"
)]
pub struct ProverTaskDbManager {
    ops: ProverTaskDbOps,
}

impl ProverTaskDbManager {
    pub fn new(pool: ThreadPool, db: Arc<impl ProverTaskDatabase + 'static>) -> Self {
        let ops = Context::new(db).into_ops(pool);
        Self { ops }
    }
}

fn db_err(e: DbError) -> ProverError {
    match e {
        DbError::EntryAlreadyExists => ProverError::TaskAlreadyExists(String::new()),
        other => ProverError::Storage(other.to_string()),
    }
}

impl TaskStore for ProverTaskDbManager {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
        let stored = self.ops.get_task_blocking(key.to_vec()).map_err(db_err)?;
        Ok(stored.map(|data| TaskRecord::from_parts(key.to_vec(), data)))
    }

    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        let (key, data) = (record.key().to_vec(), record.data().clone());
        self.ops
            .insert_task_blocking(key.clone(), data)
            .map_err(|e| match e {
                DbError::EntryAlreadyExists => ProverError::TaskAlreadyExists(format!("{:?}", key)),
                other => ProverError::Storage(other.to_string()),
            })
    }

    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        self.modify(key, |d| d.set_status(status))
    }

    fn set_retry_after(&self, key: &[u8], when_secs: SecsSinceEpoch) -> ProverResult<()> {
        self.modify(key, |d| d.set_retry_after_secs(Some(when_secs)))
    }

    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.modify(key, |d| d.set_metadata(Some(data)))
    }

    fn list_retriable(&self, now_secs: SecsSinceEpoch) -> ProverResult<Vec<TaskRecord>> {
        let items = self.ops.list_retriable_blocking(now_secs).map_err(db_err)?;
        Ok(items
            .into_iter()
            .map(|(k, d)| TaskRecord::from_parts(k, d))
            .collect())
    }

    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
        let items = self.ops.list_unfinished_blocking().map_err(db_err)?;
        Ok(items
            .into_iter()
            .map(|(k, d)| TaskRecord::from_parts(k, d))
            .collect())
    }

    fn count(&self) -> ProverResult<usize> {
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
