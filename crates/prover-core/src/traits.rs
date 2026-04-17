//! Public traits for prover-core — the single place consumers look when
//! integrating a new proof type, storage backend, or prove strategy.
//!
//! Consumer-facing traits defined here:
//! - [`ProofSpec`] — describes a proof type (task, program, input fetch).
//! - [`TaskKey`] — blanket-impl'd bounds every task identifier satisfies.
//! - [`TaskStore`] — persists task lifecycle records (opt-in).
//! - [`ReceiptStore`] — persists proof receipts by task bytes (opt-in).
//! - [`ReceiptHook`] — typed post-prove callback (opt-in).
//!
//! The prove-strategy seam (`ProveStrategy` / `ProveContext`) stays
//! crate-internal: consumers pick a backend via `ProverBuilder::.native()`
//! / `.remote()`, never by holding the trait.
//!
//! Concrete impls (`InMemoryTaskStore`, `InMemoryReceiptStore`,
//! `NativeStrategy`, `RemoteStrategy`) and supporting types
//! ([`TaskRecord`](crate::TaskRecord), [`TaskRecordData`](crate::TaskRecordData),
//! etc.) live next to their domain, not in this file.

use std::{
    fmt::{self, Debug, Display},
    hash::Hash,
    sync::Arc,
};

use async_trait::async_trait;
use zkaleido::{ProofReceiptWithMetadata, ZkVmProgram};

use crate::{
    error::ProverResult,
    task::{TaskRecord, TaskStatus},
};

// ============================================================================
// ProofSpec + TaskKey
// ============================================================================

/// Capabilities every task identifier must satisfy: equality, hashing,
/// human-readable formatting, deterministic byte encoding for storage
/// keys, and the thread-safety bounds required by background spawning.
///
/// Blanket-impl'd for any type that meets the bounds, so user task
/// types normally don't need to implement it explicitly.
///
/// Byte encoding contract: `Into<Vec<u8>> + TryFrom<Vec<u8>>` must be
/// deterministic (same task → same bytes) and round-trip lossless,
/// otherwise idempotent submit and crash recovery break. Borsh and
/// bincode are deterministic; JSON is not.
pub trait TaskKey:
    Clone + Debug + Display + Eq + Hash + Send + Sync + Into<Vec<u8>> + TryFrom<Vec<u8>> + 'static
{
}

impl<T> TaskKey for T where
    T: Clone
        + Debug
        + Display
        + Eq
        + Hash
        + Send
        + Sync
        + Into<Vec<u8>>
        + TryFrom<Vec<u8>>
        + 'static
{
}

/// Specification for a proof type.
///
/// Associates a domain task with a zkaleido program and defines how to
/// produce the program's input from that task. One impl per proof type.
///
/// # Example
///
/// ```rust,ignore
/// struct CheckpointSpec { storage: Arc<NodeStorage> }
///
/// #[async_trait]
/// impl ProofSpec for CheckpointSpec {
///     type Task = Epoch;
///     type Program = CheckpointProgram;
///
///     async fn fetch_input(&self, epoch: &Epoch) -> ProverResult<CheckpointProverInput> {
///         // storage queries ...
///     }
/// }
/// ```
#[async_trait]
pub trait ProofSpec: Send + Sync + 'static {
    /// Identifies a unit of work (e.g. `Epoch`, `ChunkTask`). See
    /// [`TaskKey`] for the bag of bounds a task identifier has to satisfy.
    type Task: TaskKey;

    /// The zkaleido program to execute. Input must be `Send` for `spawn_blocking`.
    type Program: ZkVmProgram<Input: Send + Sync> + Send + Sync + 'static;

    /// Fetch the proof input for a task.
    ///
    /// Return [`crate::ProverError::TransientFailure`] for retriable errors,
    /// [`crate::ProverError::PermanentFailure`] for fatal ones.
    async fn fetch_input(
        &self,
        task: &Self::Task,
    ) -> ProverResult<<Self::Program as ZkVmProgram>::Input>;
}

// ============================================================================
// TaskStore
// ============================================================================

/// Persistence for task records. Keyed by opaque bytes, no generics.
///
/// All methods return [`ProverResult`] so backends can surface IO/decode
/// errors to callers instead of silently discarding them. Timestamp
/// arguments are `u64` seconds since UNIX epoch, matching the stored
/// record layout.
pub trait TaskStore: Send + Sync + 'static {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>>;
    fn insert(&self, record: TaskRecord) -> ProverResult<()>;
    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()>;
    fn set_retry_after(&self, key: &[u8], when_secs: u64) -> ProverResult<()>;
    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()>;
    fn list_retriable(&self, now_secs: u64) -> ProverResult<Vec<TaskRecord>>;
    /// Every record that was submitted but hasn't reached a terminal state —
    /// Pending or Proving. Used by startup recovery to re-spawn
    /// work that was interrupted by a crash before it completed.
    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>>;
    fn count(&self) -> ProverResult<usize>;
}

/// Pass an `Arc<impl TaskStore>` straight into the builder: the wrapping Arc
/// forwards every call to the inner store. Useful when the store is a
/// shared storage manager held elsewhere in the application.
impl<T: TaskStore + ?Sized> TaskStore for Arc<T> {
    fn get(&self, key: &[u8]) -> ProverResult<Option<TaskRecord>> {
        (**self).get(key)
    }
    fn insert(&self, record: TaskRecord) -> ProverResult<()> {
        (**self).insert(record)
    }
    fn update_status(&self, key: &[u8], status: TaskStatus) -> ProverResult<()> {
        (**self).update_status(key, status)
    }
    fn set_retry_after(&self, key: &[u8], when_secs: u64) -> ProverResult<()> {
        (**self).set_retry_after(key, when_secs)
    }
    fn set_metadata(&self, key: &[u8], data: Vec<u8>) -> ProverResult<()> {
        (**self).set_metadata(key, data)
    }
    fn list_retriable(&self, now_secs: u64) -> ProverResult<Vec<TaskRecord>> {
        (**self).list_retriable(now_secs)
    }
    fn list_unfinished(&self) -> ProverResult<Vec<TaskRecord>> {
        (**self).list_unfinished()
    }
    fn count(&self) -> ProverResult<usize> {
        (**self).count()
    }
}

// ============================================================================
// ReceiptStore + ReceiptHook
// ============================================================================

/// Generic receipt persistence keyed by task bytes.
///
/// Implement this for your storage backend (sled, memory, etc.). When
/// provided to the builder, prover-core auto-stores receipts after proving
/// and PaaS exposes `get_receipt(task)` on the handle.
pub trait ReceiptStore: Send + Sync + 'static {
    fn put(&self, key: &[u8], receipt: &ProofReceiptWithMetadata) -> ProverResult<()>;
    fn get(&self, key: &[u8]) -> ProverResult<Option<ProofReceiptWithMetadata>>;
}

/// Pass an `Arc<impl ReceiptStore>` straight into the builder: the wrapping
/// Arc forwards every call to the inner store. Useful when the store is
/// shared across multiple provers (e.g. a chunk prover writes, an acct
/// prover reads from the same instance).
impl<T: ReceiptStore + ?Sized> ReceiptStore for Arc<T> {
    fn put(&self, key: &[u8], receipt: &ProofReceiptWithMetadata) -> ProverResult<()> {
        (**self).put(key, receipt)
    }
    fn get(&self, key: &[u8]) -> ProverResult<Option<ProofReceiptWithMetadata>> {
        (**self).get(key)
    }
}

/// Domain-specific hook called after a receipt is stored.
///
/// Gets the typed task (not just key bytes), so it can write to domain-specific
/// storage keyed by task identity (e.g. ProofDB keyed by `Epoch`). Most
/// consumers don't need this — only use it when you have a secondary
/// storage that needs the domain task for its key.
#[async_trait]
pub trait ReceiptHook<H: ProofSpec>: Send + Sync + 'static {
    async fn on_receipt(
        &self,
        task: &H::Task,
        receipt: &ProofReceiptWithMetadata,
    ) -> ProverResult<()>;
}

// ============================================================================
// ProveStrategy + ProveContext
// ============================================================================

/// Context passed to [`ProveStrategy::prove`] for crash-recovery metadata.
///
/// Strategies that talk to remote provers (SP1, etc.) use this to:
/// 1. Check `saved` for a proof ID from a prior crashed run
/// 2. Call `persist()` right after `start_proving()` so the ID survives a crash
///
/// Strategies that don't need recovery (e.g. native) ignore this entirely.
pub(crate) struct ProveContext {
    /// Metadata from a prior run (e.g. serialized remote ProofId).
    pub(crate) saved: Option<Vec<u8>>,
    #[cfg(feature = "remote")]
    persist_fn: Option<Box<dyn FnOnce(Vec<u8>) + Send>>,
}

impl fmt::Debug for ProveContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProveContext")
            .field("saved", &self.saved.as_ref().map(|s| s.len()))
            .finish()
    }
}

impl ProveContext {
    pub(crate) fn new(
        saved: Option<Vec<u8>>,
        _persist: impl FnOnce(Vec<u8>) + Send + 'static,
    ) -> Self {
        Self {
            saved,
            #[cfg(feature = "remote")]
            persist_fn: Some(Box::new(_persist)),
        }
    }

    /// Persist metadata for crash recovery. Call this right after obtaining
    /// a remote proof ID, before starting the poll loop.
    #[cfg(feature = "remote")]
    pub(crate) fn persist(&mut self, data: Vec<u8>) {
        if let Some(f) = self.persist_fn.take() {
            f(data);
        }
    }
}

/// Blocking prove operation. Called inside `spawn_blocking`.
///
/// Implementations capture the zkVM host internally. The `Host` type
/// is erased when stored as `Arc<dyn ProveStrategy<H>>` in the prover.
pub(crate) trait ProveStrategy<H: ProofSpec>: Send + Sync + 'static {
    fn prove(
        &self,
        input: &<H::Program as ZkVmProgram>::Input,
        ctx: ProveContext,
    ) -> ProverResult<ProofReceiptWithMetadata>;
}
