//! Sled-backed persistent task store.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use borsh::{BorshDeserialize, BorshSerialize};

use super::traits::{TaskRecord, TaskStore};
use crate::{
    error::{ProverError, ProverResult},
    task::TaskStatus,
};

/// Serializable form of [`TaskRecord`] for sled storage.
#[derive(BorshSerialize, BorshDeserialize)]
struct StoredRecord {
    status: TaskStatus,
    /// Seconds since UNIX epoch.
    updated_at_secs: u64,
    /// Seconds since UNIX epoch, if set.
    retry_after_secs: Option<u64>,
    /// Opaque strategy metadata (e.g. remote ProofId).
    metadata: Option<Vec<u8>>,
}

impl StoredRecord {
    fn from_record(record: &TaskRecord) -> Self {
        Self {
            status: record.status().clone(),
            updated_at_secs: system_time_to_secs(SystemTime::now()),
            retry_after_secs: record.retry_after().map(system_time_to_secs),
            metadata: record.metadata().map(|m| m.to_vec()),
        }
    }

    fn into_record(self, key: Vec<u8>) -> TaskRecord {
        let mut r = TaskRecord::new(key, self.status);
        if let Some(secs) = self.retry_after_secs {
            r.set_retry_after(Some(secs_to_system_time(secs)));
        }
        if let Some(data) = self.metadata {
            r.set_metadata(Some(data));
        }
        r
    }
}

fn system_time_to_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn secs_to_system_time(secs: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs)
}

/// Persistent [`TaskStore`] backed by sled.
#[derive(Debug)]
pub struct SledTaskStore {
    tree: sled::Tree,
}

impl SledTaskStore {
    /// Open a task store using the given sled tree.
    pub fn new(tree: sled::Tree) -> Self {
        Self { tree }
    }

    /// Open a task store from a sled database, using "prover_tasks" as the tree name.
    pub fn open(db: &sled::Db) -> ProverResult<Self> {
        let tree = db
            .open_tree("prover_tasks")
            .map_err(|e| ProverError::Internal(e.into()))?;
        Ok(Self::new(tree))
    }

    fn get_stored(&self, key: &[u8]) -> ProverResult<Option<StoredRecord>> {
        match self.tree.get(key) {
            Ok(Some(bytes)) => {
                let record = borsh::from_slice(&bytes)
                    .map_err(|e| ProverError::Internal(anyhow::anyhow!("deserialize: {e}")))?;
                Ok(Some(record))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(ProverError::Internal(e.into())),
        }
    }

    fn put_stored(&self, key: &[u8], record: &StoredRecord) -> ProverResult<()> {
        let bytes = borsh::to_vec(record)
            .map_err(|e| ProverError::Internal(anyhow::anyhow!("serialize: {e}")))?;
        self.tree
            .insert(key, bytes)
            .map_err(|e| ProverError::Internal(e.into()))?;
        Ok(())
    }

    fn modify<F>(&self, key: &[u8], f: F) -> ProverResult<()>
    where
        F: FnOnce(&mut StoredRecord),
    {
        let mut record = self
            .get_stored(key)?
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?;
        f(&mut record);
        record.updated_at_secs = system_time_to_secs(SystemTime::now());
        self.put_stored(key, &record)
    }
}

impl TaskStore for SledTaskStore {
    fn get(&self, key: &[u8]) -> Option<TaskRecord> {
        self.get_stored(key)
            .ok()
            .flatten()
            .map(|r| r.into_record(key.to_vec()))
    }

    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        if self
            .tree
            .contains_key(record.key())
            .unwrap_or(false)
        {
            return Err(ProverError::TaskAlreadyExists(format!("{:?}", record.key())));
        }
        let stored = StoredRecord::from_record(&record);
        self.put_stored(record.key(), &stored)
    }

    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        self.modify(key, |r| r.status = status)
    }

    fn set_retry_after(&self, key: &[u8], when: SystemTime) -> ProverResult<()> {
        self.modify(key, |r| {
            r.retry_after_secs = Some(system_time_to_secs(when));
        })
    }

    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.modify(key, |r| r.metadata = Some(data))
    }

    fn list_retriable(&self, now: SystemTime) -> Vec<TaskRecord> {
        let now_secs = system_time_to_secs(now);
        self.tree
            .iter()
            .filter_map(|item| {
                let (key, val) = item.ok()?;
                let record: StoredRecord = borsh::from_slice(&val).ok()?;
                if record.status.is_retriable()
                    && record.retry_after_secs.is_some_and(|t| t <= now_secs)
                {
                    Some(record.into_record(key.to_vec()))
                } else {
                    None
                }
            })
            .collect()
    }

    fn list_in_progress(&self) -> Vec<TaskRecord> {
        self.tree
            .iter()
            .filter_map(|item| {
                let (key, val) = item.ok()?;
                let record: StoredRecord = borsh::from_slice(&val).ok()?;
                if record.status.is_in_progress() {
                    Some(record.into_record(key.to_vec()))
                } else {
                    None
                }
            })
            .collect()
    }

    fn count(&self) -> usize {
        self.tree.len()
    }
}
