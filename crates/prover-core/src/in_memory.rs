//! In-memory [`TaskStore`] and [`ReceiptStore`] impls for tests and dev.
//!
//! For prod, users implement their own [`TaskStore`] / [`ReceiptStore`].

use std::collections::HashMap;

use parking_lot::RwLock;
use zkaleido::ProofReceiptWithMetadata;

use crate::{
    error::{ProverError, ProverResult},
    task::{TaskRecord, TaskStatus},
    traits::{ReceiptStore, TaskStore},
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
            .data_mut()
            .set_status(status);
        Ok(())
    }

    fn set_retry_after(&self, key: &[u8], when_secs: u64) -> ProverResult<()> {
        self.records
            .write()
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .data_mut()
            .set_retry_after_secs(Some(when_secs));
        Ok(())
    }

    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.records
            .write()
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .data_mut()
            .set_metadata(Some(data));
        Ok(())
    }

    fn list_retriable(&self, now_secs: u64) -> ProverResult<Vec<TaskRecord>> {
        Ok(self
            .records
            .read()
            .values()
            .filter(|r| {
                r.status().is_retriable() && r.retry_after_secs().is_some_and(|t| t <= now_secs)
            })
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

#[derive(Debug, Default)]
pub struct InMemoryReceiptStore {
    receipts: RwLock<HashMap<Vec<u8>, ProofReceiptWithMetadata>>,
}

impl InMemoryReceiptStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ReceiptStore for InMemoryReceiptStore {
    fn put(&self, key: &[u8], receipt: &ProofReceiptWithMetadata) -> ProverResult<()> {
        self.receipts.write().insert(key.to_vec(), receipt.clone());
        Ok(())
    }

    fn get(&self, key: &[u8]) -> ProverResult<Option<ProofReceiptWithMetadata>> {
        Ok(self.receipts.read().get(key).cloned())
    }
}
