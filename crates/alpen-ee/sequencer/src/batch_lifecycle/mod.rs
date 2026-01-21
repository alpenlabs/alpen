//! Batch lifecycle manager for managing batch state transitions.
//!
//! The batch lifecycle manager bridges the gap between the batch builder
//! (which creates `Sealed` batches) and the update submitter (which consumes
//! `ProofReady` batches). It manages the intermediate lifecycle states:
//!
//! ```text
//! Sealed → DaPending → DaComplete → ProofPending → ProofReady
//! ```
//!
//! # Architecture
//!
//! The lifecycle manager is built around a state machine that processes batches
//! sequentially through their lifecycle. It uses:
//!
//! - [`BatchDaProvider`] trait for posting DA and checking confirmation status
//! - [`BatchProver`] trait for requesting and checking proof generation
//! - [`BatchStorage`] for persisting batch status updates
//!
//! # Usage
//!
//! ```ignore
//! use alpen_ee_sequencer::{
//!     create_batch_lifecycle, init_lifecycle_state,
//! };
//!
//! // Initialize state from storage
//! let state = init_lifecycle_state(&batch_storage).await?;
//!
//! // Create the task
//! let (handle, task) = create_batch_lifecycle_task(
//!     initial_batch_id,
//!     state,
//!     sealed_batch_rx,
//!     da_provider,
//!     prover,
//!     batch_storage,
//! );
//!
//! // Use handle to watch for proof-ready batches
//! let watcher = handle.proof_ready_watcher();
//!
//! // Run the task
//! task.await;
//! ```
//!
//! # Reorg Handling
//!
//! The lifecycle manager uses a passive reorg handling strategy. It relies on
//! the batch builder to call `revert_batches()` on reorg. When the target batch
//! index (from sealed_batch notifications) moves backwards, the lifecycle manager
//! detects this and resets its internal state accordingly.
//!
//! [`BatchDaProvider`]: alpen_ee_common::BatchDaProvider
//! [`BatchProver`]: alpen_ee_common::BatchProver
//! [`BatchStorage`]: alpen_ee_common::BatchStorage

mod ctx;
mod handle;
mod reorg;
mod state;
mod task;
#[cfg(test)]
pub(crate) mod test_utils;

pub use handle::{create_batch_lifecycle_task, BatchLifecycleHandle};
pub use state::{init_lifecycle_state, BatchLifecycleState};
