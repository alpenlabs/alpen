//! Sled-backed storage managers for the EE prover.
//!
//! Three managers, all wrapping the shared [`EeProverDbSled`]:
//!
//! - [`EeProverTaskDbManager`] — impls `paas::TaskStore`. Shared across
//!   chunk + acct provers via the kind-tagged task-key encoding (see
//!   `CHUNK_TASK_TAG` / `BATCH_TASK_TAG`).
//! - [`EeChunkReceiptStore`] — impls `paas::ReceiptStore`. The chunk
//!   prover writes here; the acct `fetch_input` reads from here.
//! - [`EeBatchProofDbManager`] — typed API keyed by [`BatchId`]; the
//!   outer (acct) prover writes here via its `ReceiptHook`, and the
//!   `BatchProver::get_proof(proof_id)` lookup is served from here.
//!
//! Parallels the OL pattern (`strata_storage::managers::{ProverTaskDbManager,
//! CheckpointProofDbManager}`) but lives in its own sled instance
//! under the alpen-client datadir — no cross-wiring with OL's
//! checkpoint storage.
//!
//! All methods are synchronous. Sled ops are fast; PAAS drives these
//! from a background tick loop and its `ReceiptHook` is already async,
//! so calls from async contexts don't block meaningfully. No threadpool
//! layer for now — add one if this shows up in profiling.

use std::sync::Arc;

use alpen_ee_common::{BatchId, Proof, ProofId};
use alpen_ee_database::EeProverDbSled;
use strata_db_types::{errors::DbError, traits::ProverTaskDatabase};
use strata_paas::{
    ProverError, ProverResult, ReceiptStore, TaskRecord, TaskRecordData, TaskStatus, TaskStore,
};
use zkaleido::ProofReceiptWithMetadata;

fn db_err(e: DbError) -> ProverError {
    match e {
        DbError::EntryAlreadyExists => ProverError::TaskAlreadyExists(String::new()),
        other => ProverError::Storage(other.to_string()),
    }
}

/// Sled-backed shared prover task store.
///
/// Both chunk and acct provers hold an `Arc<Self>` and pass it to
/// `ProverBuilder::task_store(...)`. Task keys carry a single-byte
/// kind tag (`b'c'` / `b'a'`) inside their `Task::into()` encoding,
/// so entries from the two provers don't collide in the shared tree.
#[derive(Debug, Clone)]
pub(crate) struct EeProverTaskDbManager {
    db: Arc<EeProverDbSled>,
}

impl EeProverTaskDbManager {
    pub(crate) fn new(db: Arc<EeProverDbSled>) -> Self {
        Self { db }
    }

    fn modify<F>(&self, key: &[u8], f: F) -> ProverResult<()>
    where
        F: FnOnce(&mut TaskRecordData),
    {
        let mut data = self
            .db
            .get_task(key.to_vec())
            .map_err(db_err)?
            .ok_or_else(|| ProverError::TaskNotFound(format!("{:?}", key)))?;
        f(&mut data);
        self.db.put_task(key.to_vec(), data).map_err(db_err)
    }
}

impl TaskStore for EeProverTaskDbManager {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
        let stored = self.db.get_task(key.to_vec()).map_err(db_err)?;
        Ok(stored.map(|data| TaskRecord::from_parts(key.to_vec(), data)))
    }

    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        let (key, data) = (record.key().to_vec(), record.data().clone());
        self.db.insert_task(key.clone(), data).map_err(|e| match e {
            DbError::EntryAlreadyExists => ProverError::TaskAlreadyExists(format!("{:?}", key)),
            other => ProverError::Storage(other.to_string()),
        })
    }

    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        self.modify(key, |d| d.set_status(status))
    }

    fn set_retry_after(&self, key: &[u8], when_secs: u64) -> ProverResult<()> {
        self.modify(key, |d| d.set_retry_after_secs(Some(when_secs)))
    }

    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        self.modify(key, |d| d.set_metadata(Some(data)))
    }

    fn list_retriable(&self, now_secs: u64) -> ProverResult<Vec<TaskRecord>> {
        let items = self.db.list_retriable(now_secs).map_err(db_err)?;
        Ok(items
            .into_iter()
            .map(|(k, d)| TaskRecord::from_parts(k, d))
            .collect())
    }

    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
        let items = self.db.list_unfinished().map_err(db_err)?;
        Ok(items
            .into_iter()
            .map(|(k, d)| TaskRecord::from_parts(k, d))
            .collect())
    }

    fn count(&self) -> ProverResult<usize> {
        self.db.count_tasks().map_err(db_err)
    }
}

/// Sled-backed chunk receipt store.
///
/// Keyed by chunk task bytes (matches paas's `ReceiptStore`). The chunk
/// prover writes via its auto-store after proving; `AcctSpec::fetch_input`
/// reads via `collect_chunk_inputs_for_batch`.
#[derive(Debug, Clone)]
pub(crate) struct EeChunkReceiptStore {
    db: Arc<EeProverDbSled>,
}

impl EeChunkReceiptStore {
    pub(crate) fn new(db: Arc<EeProverDbSled>) -> Self {
        Self { db }
    }
}

impl ReceiptStore for EeChunkReceiptStore {
    fn put(&self, key: &[u8], receipt: &ProofReceiptWithMetadata) -> ProverResult<()> {
        self.db
            .put_chunk_receipt(key.to_vec(), receipt.clone())
            .map_err(db_err)
    }

    fn get(&self, key: &[u8]) -> ProverResult<Option<ProofReceiptWithMetadata>> {
        self.db.get_chunk_receipt(key).map_err(db_err)
    }
}

/// Typed outer-proof storage keyed by [`BatchId`].
///
/// Sled-backed replacement for the earlier in-memory `HashMap` version.
/// The `AcctReceiptHook` writes here; `PaasBatchProver::get_proof(proof_id)`
/// serves OL submission from the secondary `ProofId → BatchId` index.
#[derive(Debug, Clone)]
pub(crate) struct EeBatchProofDbManager {
    db: Arc<EeProverDbSled>,
}

impl EeBatchProofDbManager {
    pub(crate) fn new(db: Arc<EeProverDbSled>) -> Self {
        Self { db }
    }

    /// `ProofId` for a batch — its `last_block` hash. Stable across
    /// in-memory and sled storage layers so the secondary index is
    /// a 1:1 map with the manager's public API.
    pub(crate) fn proof_id_for(batch_id: BatchId) -> ProofId {
        batch_id.last_block()
    }

    pub(crate) fn put_proof(
        &self,
        batch_id: BatchId,
        receipt: ProofReceiptWithMetadata,
    ) -> ProverResult<()> {
        self.db.put_acct_proof(batch_id, receipt).map_err(db_err)
    }

    pub(crate) fn has_proof(&self, batch_id: BatchId) -> bool {
        // sled errors surface as "not found"; callers treat this as a
        // storage-level concern and log separately.
        self.db.has_acct_proof(batch_id).unwrap_or(false)
    }

    pub(crate) fn get_proof_by_id(&self, proof_id: ProofId) -> Option<Proof> {
        let receipt = self.db.get_acct_proof_by_id(proof_id).ok().flatten()?;
        Some(Proof::from_vec(
            receipt.receipt().proof().as_bytes().to_vec(),
        ))
    }
}
