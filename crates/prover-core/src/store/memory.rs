//! In-memory task store. Default for tests and dev.

use std::{collections::HashMap, sync::RwLock, time::SystemTime};

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
    fn get(&self, key: &[u8]) -> Option<TaskRecord> {
        self.records.read().expect("lock").get(key).cloned()
    }

    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        let mut map = self.records.write().expect("lock");
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
            .expect("lock")
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .update_status(status);
        Ok(())
    }

    fn set_retry_after(&self, key: &[u8], when: SystemTime) -> ProverResult<()> {
        self.records
            .write()
            .expect("lock")
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .set_retry_after(Some(when));
        Ok(())
    }

    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.records
            .write()
            .expect("lock")
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .set_metadata(Some(data));
        Ok(())
    }

    fn list_retriable(&self, now: SystemTime) -> Vec<TaskRecord> {
        self.records
            .read()
            .expect("lock")
            .values()
            .filter(|r| r.status().is_retriable() && r.retry_after().is_some_and(|t| t <= now))
            .cloned()
            .collect()
    }

    fn list_in_progress(&self) -> Vec<TaskRecord> {
        self.records
            .read()
            .expect("lock")
            .values()
            .filter(|r| r.status().is_in_progress())
            .cloned()
            .collect()
    }

    fn count(&self) -> usize {
        self.records.read().expect("lock").len()
    }
}
