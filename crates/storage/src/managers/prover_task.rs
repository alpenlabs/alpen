//! High-level manager for the prover task store.
//!
//! Wraps [`ProverTaskDatabase`] with a threadpool + instrumentation, and
//! implements [`strata_paas::TaskStore`] directly so the integrated prover
//! service can consume the manager as its persistent task store without
//! any extra adapter. The conversion between
//! [`strata_db_types::types::PersistedTaskRecord`] and
//! [`strata_paas::TaskRecordData`] lives here.

use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use strata_db_types::{traits::ProverTaskDatabase, types::PersistedTaskRecord, errors::DbError};
use strata_paas::{
    ProverError, ProverResult, TaskRecord, TaskRecordData, TaskStatus, TaskStore,
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

// ---- helpers ----------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn secs_to_system_time(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

fn system_time_to_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn persisted_to_task_record(key: Vec<u8>, p: PersistedTaskRecord) -> TaskRecord {
    let mut r = TaskRecord::new(key, p.status);
    if let Some(secs) = p.retry_after_secs {
        r.data_mut()
            .set_retry_after(Some(secs_to_system_time(secs)));
    }
    if let Some(bytes) = p.metadata {
        r.data_mut().set_metadata(Some(bytes));
    }
    r
}

fn data_to_persisted(data: &TaskRecordData) -> PersistedTaskRecord {
    PersistedTaskRecord {
        status: data.status().clone(),
        updated_at_secs: now_secs(),
        retry_after_secs: data.retry_after().map(system_time_to_secs),
        metadata: data.metadata().map(|m| m.to_vec()),
    }
}

fn db_err(e: DbError) -> ProverError {
    match e {
        DbError::EntryAlreadyExists => ProverError::TaskAlreadyExists(String::new()),
        other => ProverError::Storage(other.to_string()),
    }
}

// ---- TaskStore impl ---------------------------------------------------------

impl TaskStore for ProverTaskDbManager {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
        let stored = self.ops.get_task_blocking(key.to_vec()).map_err(db_err)?;
        Ok(stored.map(|p| persisted_to_task_record(key.to_vec(), p)))
    }

    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        let key = record.key().to_vec();
        let persisted = data_to_persisted(record.data());
        self.ops
            .insert_task_blocking(key, persisted)
            .map_err(|e| match e {
                DbError::EntryAlreadyExists => {
                    ProverError::TaskAlreadyExists(format!("{:?}", record.key()))
                }
                other => ProverError::Storage(other.to_string()),
            })
    }

    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        self.modify(key, |p| p.status = status)
    }

    fn set_retry_after(&self, key: &[u8], when: SystemTime) -> ProverResult<()> {
        let secs = system_time_to_secs(when);
        self.modify(key, |p| p.retry_after_secs = Some(secs))
    }

    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.modify(key, |p| p.metadata = Some(data))
    }

    fn list_retriable(&self, now: SystemTime) -> ProverResult<Vec<TaskRecord>> {
        let items = self
            .ops
            .list_retriable_blocking(system_time_to_secs(now))
            .map_err(db_err)?;
        Ok(items
            .into_iter()
            .map(|(k, p)| persisted_to_task_record(k, p))
            .collect())
    }

    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
        let items = self.ops.list_unfinished_blocking().map_err(db_err)?;
        Ok(items
            .into_iter()
            .map(|(k, p)| persisted_to_task_record(k, p))
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
        F: FnOnce(&mut PersistedTaskRecord),
    {
        let mut persisted = self
            .ops
            .get_task_blocking(key.to_vec())
            .map_err(db_err)?
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?;
        f(&mut persisted);
        persisted.updated_at_secs = now_secs();
        self.ops
            .put_task_blocking(key.to_vec(), persisted)
            .map_err(db_err)
    }
}
