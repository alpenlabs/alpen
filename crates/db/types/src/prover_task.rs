//! Prover task database interface.

#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_paas::TaskRecordData;

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Database interface backing [`strata_paas::TaskStore`] for the integrated
/// prover service.
///
/// Keyed by the serialized `ProofSpec::Task` bytes — same contract as the
/// in-memory `TaskStore`. All methods are synchronous and expected to be
/// called through a blocking threadpool by the `strata_storage` manager.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:prover_task")
)]
pub trait ProverTaskDatabase: Send + Sync + 'static {
    /// Fetch a record by key. `None` if the key is absent.
    fn get_task(&self, key: Vec<u8>) -> DbResult<Option<TaskRecordData>>;

    /// Insert a new record. Fails with `DbError::EntryAlreadyExists` if
    /// the key is already present — implementations must do this atomically
    /// (e.g. `compare_and_swap(None, Some)`).
    fn insert_task(&self, key: Vec<u8>, record: TaskRecordData) -> DbResult<()>;

    /// Upsert a record — overwrites any existing entry under the key.
    fn put_task(&self, key: Vec<u8>, record: TaskRecordData) -> DbResult<()>;

    /// Removes a task record. Returns `true` if the key existed prior to the
    /// call, `false` otherwise.
    ///
    /// Intended for offline admin tooling (e.g. `strata-dbtool`) — the
    /// runtime task lifecycle is driven by status transitions, not deletion.
    fn delete_task(&self, key: Vec<u8>) -> DbResult<bool>;

    /// All records where `status` is retriable and `retry_after_secs <= now_secs`.
    fn list_retriable(&self, now_secs: u64) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// All records whose status is not yet terminal (Pending / Proving).
    fn list_unfinished(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// Every record in the store, in implementation-defined order.
    ///
    /// Intended for offline admin tooling — the runtime path uses the
    /// filtered iterators above to avoid scanning terminal entries.
    fn list_all_tasks(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// Number of records in the store.
    fn count_tasks(&self) -> DbResult<usize>;
}
