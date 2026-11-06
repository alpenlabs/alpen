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

    fn schedule_rescan(
        &self,
        key: &[u8],
        status: TaskStatus,
        retry_after_secs: u64,
    ) -> ProverResult<()> {
        let mut map = self.records.write();
        let data = map
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .data_mut();
        data.set_status(status);
        data.set_retry_after_secs(Some(retry_after_secs));
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

    fn clear_metadata(&self, key: &[u8]) -> ProverResult<()> {
        self.records
            .write()
            .get_mut(key)
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?
            .data_mut()
            .set_metadata(None);
        Ok(())
    }

    fn list_retriable(&self, now_secs: u64) -> ProverResult<Vec<TaskRecord>> {
        Ok(self
            .records
            .read()
            .values()
            .filter(|r| r.data().is_due_for_rescan(now_secs))
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::AttemptCounts;

    fn transient() -> TaskStatus {
        TaskStatus::TransientFailure {
            counts: AttemptCounts::default(),
            error: "retry".into(),
        }
    }

    #[test]
    fn list_retriable_treats_a_missing_retry_after_as_due() {
        let store = InMemoryTaskStore::new();
        store
            .insert(TaskRecord::new(vec![1], TaskStatus::Pending))
            .unwrap();
        store.update_status(&[1], transient()).unwrap();
        store
            .insert(TaskRecord::new(vec![2], TaskStatus::Pending))
            .unwrap();
        store.update_status(&[2], transient()).unwrap();
        store.set_retry_after(&[2], 10).unwrap();

        // The record without a timestamp is due at any time; the other waits.
        let keys = |now| {
            let mut keys: Vec<_> = store
                .list_retriable(now)
                .unwrap()
                .into_iter()
                .map(|r| r.key().to_vec())
                .collect();
            keys.sort();
            keys
        };
        assert_eq!(keys(0), vec![vec![1]]);
        assert_eq!(keys(10), vec![vec![1], vec![2]]);
    }

    #[test]
    fn schedule_rescan_writes_status_and_retry_after_together() {
        let store = InMemoryTaskStore::new();
        store
            .insert(TaskRecord::new(vec![1], TaskStatus::Pending))
            .unwrap();
        store.schedule_rescan(&[1], transient(), 42).unwrap();
        let record = store.get(&[1]).unwrap().unwrap();
        assert_eq!(record.status(), &transient());
        assert_eq!(record.retry_after_secs(), Some(42));
        assert!(matches!(
            store.schedule_rescan(&[9], transient(), 42),
            Err(ProverError::TaskNotFound(_))
        ));
    }

    /// A store that only has the two single-field writes, with either one
    /// failing on demand, running the trait's default `schedule_rescan`.
    struct TwoWriteStore {
        inner: InMemoryTaskStore,
        fail_status: AtomicBool,
        fail_retry_after: AtomicBool,
    }

    impl TaskStore for TwoWriteStore {
        fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
            self.inner.get(key)
        }
        fn insert(&self, record: TaskRecord) -> ProverResult<()> {
            self.inner.insert(record)
        }
        fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
            if self.fail_status.swap(false, Ordering::SeqCst) {
                return Err(ProverError::Storage("status write failed".into()));
            }
            self.inner.update_status(key, status)
        }
        fn set_retry_after(&self, key: &[u8], when_secs: u64) -> ProverResult<()> {
            if self.fail_retry_after.swap(false, Ordering::SeqCst) {
                return Err(ProverError::Storage("retry_after write failed".into()));
            }
            self.inner.set_retry_after(key, when_secs)
        }
        fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
            self.inner.set_metadata(key, data)
        }
        fn clear_metadata(&self, key: &[u8]) -> ProverResult<()> {
            self.inner.clear_metadata(key)
        }
        fn list_retriable(&self, now_secs: u64) -> ProverResult<Vec<TaskRecord>> {
            self.inner.list_retriable(now_secs)
        }
        fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
            self.inner.list_unfinished()
        }
        fn count(&self) -> ProverResult<usize> {
            self.inner.count()
        }
    }

    #[test]
    fn default_schedule_rescan_never_leaves_a_rescan_status_without_its_timestamp() {
        for (fail_status, fail_retry_after) in [(false, true), (true, false)] {
            let store = TwoWriteStore {
                inner: InMemoryTaskStore::new(),
                fail_status: AtomicBool::new(fail_status),
                fail_retry_after: AtomicBool::new(fail_retry_after),
            };
            let proving = TaskStatus::Proving {
                counts: AttemptCounts::default(),
            };
            store
                .insert(TaskRecord::new(vec![1], proving.clone()))
                .unwrap();

            assert!(store.schedule_rescan(&[1], transient(), 42).is_err());

            // Whichever write failed, the record is still Proving, so startup
            // recovery re-spawns it, and it is never a rescan status that the
            // scanner would have to find by its missing timestamp.
            let record = store.get(&[1]).unwrap().unwrap();
            assert_eq!(record.status(), &proving);
            assert_eq!(store.list_unfinished().unwrap().len(), 1);
            assert_eq!(
                record.retry_after_secs(),
                if fail_retry_after { None } else { Some(42) }
            );

            // The next attempt lands both.
            store.schedule_rescan(&[1], transient(), 43).unwrap();
            let record = store.get(&[1]).unwrap().unwrap();
            assert_eq!(record.status(), &transient());
            assert_eq!(record.retry_after_secs(), Some(43));
        }
    }
}
