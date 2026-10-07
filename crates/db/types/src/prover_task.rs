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
/// Keyed by physical task bytes. Service adapters may qualify a logical
/// `ProofSpec::Task` key with a namespace before storing it. All methods are synchronous and expected to be
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
    /// Deletes by key without decoding the stored record, including malformed values.
    /// Used by startup reconciliation and offline admin tooling.
    fn delete_task(&self, key: Vec<u8>) -> DbResult<bool>;

    /// All records where `status` is retriable and `retry_after_secs <= now_secs`.
    fn list_retriable(&self, now_secs: u64) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// All records whose status is not yet terminal (Pending / Proving).
    fn list_unfinished(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// Lists due retry or blocked records with `spec_prefix` followed by `task_key_len` task-key bytes.
    fn list_retriable_with_prefix(
        &self,
        spec_prefix: [u8; 4],
        task_key_len: usize,
        now_secs: u64,
    ) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// Lists Pending or Proving records with `spec_prefix` followed by `task_key_len` task-key bytes.
    fn list_unfinished_with_prefix(
        &self,
        spec_prefix: [u8; 4],
        task_key_len: usize,
    ) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// Every record in the store, in implementation-defined order.
    ///
    /// Used by offline admin tooling.
    fn list_all_tasks(&self) -> DbResult<Vec<(Vec<u8>, TaskRecordData)>>;

    /// Counts keys with `spec_prefix` followed by `task_key_len` task-key bytes.
    ///
    /// The four-byte prefix encodes the OL spec version.
    /// Streams matching records without collecting them, including terminal tasks.
    /// The length check excludes legacy or malformed keys that share the spec prefix.
    fn count_tasks_with_prefix(&self, spec_prefix: [u8; 4], task_key_len: usize)
        -> DbResult<usize>;

    /// Number of records in the store.
    fn count_tasks(&self) -> DbResult<usize>;
}
