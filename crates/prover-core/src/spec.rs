//! Proof specification trait.
//!
//! Defines what to prove (program), what identifies a task, and how to fetch the input.
//! Receipt storage and domain hooks are separate opt-in concerns
//! (see [`ReceiptStore`](crate::ReceiptStore) and [`ReceiptHook`](crate::ReceiptHook)).

use std::{
    fmt::{Debug, Display},
    hash::Hash,
};

use async_trait::async_trait;
use zkaleido::ZkVmProgram;

use crate::error::ProverResult;

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
