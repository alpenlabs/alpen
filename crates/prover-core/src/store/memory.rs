//! In-memory task store. Default for tests and dev.

use std::{collections::HashMap, time::SystemTime};

use parking_lot::RwLock;

use super::traits::{TaskRecord, TaskStore};
use crate::{
    error::{ProverError, ProverResult},
    task::TaskStatus,
};

#[derive(Debug, Default)]
pub struct InMemoryTaskStore {
    records: RwLock<HashMap<Vec<u8>, TaskRecord>>,
}

impl InMemoryTaskStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TaskStore for InMemoryTaskStore {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
        Ok(self.records.read().get(key).cloned())
    }

    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        let mut map = self.records.write();
        if map.contains_key(record.key()) {
            return Err(ProverError::TaskAlreadyExists(format!(
                "{:?}",
                record.key()
            )));
        }
        map.insert(record.key().to_vec(), record);
        Ok(())
    }

    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        self.records
            .write()
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .update_status(status);
        Ok(())
    }

    fn set_retry_after(&self, key: &[u8], when: SystemTime) -> ProverResult<()> {
        self.records
            .write()
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .set_retry_after(Some(when));
        Ok(())
    }

    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.records
            .write()
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .set_metadata(Some(data));
        Ok(())
    }

    fn list_retriable(&self, now: SystemTime) -> ProverResult<Vec<TaskRecord>> {
        Ok(self
            .records
            .read()
            .values()
            .filter(|r| r.status().is_retriable() && r.retry_after().is_some_and(|t| t <= now))
            .cloned()
            .collect())
    }

    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
        Ok(self
            .records
            .read()
            .values()
            .filter(|r| r.status().is_unfinished())
            .cloned()
            .collect())
    }

    fn count(&self) -> ProverResult<usize> {
        Ok(self.records.read().len())
    }
}
