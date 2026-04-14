//! Sled-backed persistent task store.
//!
//! Records are encoded as JSON via serde — readable by `sled-cli` or any
//! JSON-aware tool. Compactness is not a goal here; a handful of bytes per
//! record comes from timestamps and a short enum tag. Borsh was considered
//! and rejected: field-order-sensitive encoding and no self-describing
//! evolution story for a store that lives across releases.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::traits::{TaskRecord, TaskStore};
use crate::{
    error::{ProverError, ProverResult},
    task::TaskStatus,
};

/// Serializable form of [`TaskRecord`] for sled storage.
#[derive(Serialize, Deserialize)]
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
            r.data_mut()
                .set_retry_after(Some(secs_to_system_time(secs)));
        }
        if let Some(data) = self.metadata {
            r.data_mut().set_metadata(Some(data));
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

fn encode(record: &StoredRecord) -> ProverResult<Vec<u8>> {
    serde_json::to_vec(record).map_err(|e| ProverError::Codec(format!("encode record: {e}")))
}

fn decode(bytes: &[u8]) -> ProverResult<StoredRecord> {
    serde_json::from_slice(bytes).map_err(|e| ProverError::Codec(format!("decode record: {e}")))
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
            .map_err(|e| ProverError::Storage(e.to_string()))?;
        Ok(Self::new(tree))
    }

    /// Atomically read-modify-write the record at `key`.
    ///
    /// Uses sled's [`Tree::update_and_fetch`] so concurrent modifiers on the
    /// same key don't clobber each other. The closure runs inside sled's
    /// update loop and may be invoked more than once on contention.
    fn modify_record<F>(&self, key: &[u8], f: F) -> ProverResult<()>
    where
        F: Fn(&mut StoredRecord),
    {
        // Capture decode/encode errors out of the closure via an Option, then
        // surface them after the atomic op returns.
        let mut io_err: Option<ProverError> = None;
        let outcome = self.tree.update_and_fetch(key, |old| {
            let bytes = old?;
            let mut record = match decode(bytes) {
                Ok(r) => r,
                Err(e) => {
                    io_err = Some(e);
                    return Some(bytes.to_vec());
                }
            };
            f(&mut record);
            record.updated_at_secs = system_time_to_secs(SystemTime::now());
            match encode(&record) {
                Ok(encoded) => Some(encoded),
                Err(e) => {
                    io_err = Some(e);
                    Some(bytes.to_vec())
                }
            }
        });

        if let Some(e) = io_err {
            return Err(e);
        }
        match outcome {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(ProverError::TaskNotFound(format!("{:?}", key))),
            Err(e) => Err(ProverError::Storage(e.to_string())),
        }
    }

    /// Scan the tree, surfacing IO and decode errors.
    fn scan<F>(&self, keep: F) -> ProverResult<Vec<TaskRecord>>
    where
        F: Fn(&StoredRecord) -> bool,
    {
        let mut out = Vec::new();
        for item in self.tree.iter() {
            let (key, val) = item.map_err(|e| ProverError::Storage(e.to_string()))?;
            let record = decode(&val)?;
            if keep(&record) {
                out.push(record.into_record(key.to_vec()));
            }
        }
        Ok(out)
    }
}

impl TaskStore for SledTaskStore {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
        match self
            .tree
            .get(key)
            .map_err(|e| ProverError::Storage(e.to_string()))?
        {
            Some(bytes) => {
                let record = decode(&bytes)?;
                Ok(Some(record.into_record(key.to_vec())))
            }
            None => Ok(None),
        }
    }

    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        // Atomic "only if absent" via compare_and_swap.
        let key = record.key().to_vec();
        let bytes = encode(&StoredRecord::from_record(&record))?;
        let res = self
            .tree
            .compare_and_swap::<_, &[u8], _>(&key, None, Some(bytes))
            .map_err(|e| ProverError::Storage(e.to_string()))?;
        match res {
            Ok(()) => Ok(()),
            Err(_) => Err(ProverError::TaskAlreadyExists(format!("{:?}", record.key()))),
        }
    }

    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        self.modify_record(key, |r| r.status = status.clone())
    }

    fn set_retry_after(&self, key: &[u8], when: SystemTime) -> ProverResult<()> {
        let secs = system_time_to_secs(when);
        self.modify_record(key, |r| r.retry_after_secs = Some(secs))
    }

    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.modify_record(key, |r| r.metadata = Some(data.clone()))
    }

    fn list_retriable(&self, now: SystemTime) -> ProverResult<Vec<TaskRecord>> {
        let now_secs = system_time_to_secs(now);
        self.scan(|r| {
            r.status.is_retriable() && r.retry_after_secs.is_some_and(|t| t <= now_secs)
        })
    }

    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
        self.scan(|r| r.status.is_unfinished())
    }

    fn count(&self) -> ProverResult<usize> {
        Ok(self.tree.len())
    }
}
