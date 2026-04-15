//! Single-proof-type proving engine with zkaleido integration.
//!
//! Each [`Prover`] wraps one [`ProofSpec`] and a prove strategy (native or remote).
//! The spec fetches inputs. Receipt storage and domain hooks are opt-in.
//! The prover runs the zkVM program via the strategy.
//!
//! Public traits consumers implement live in the `traits` module;
//! everything else (concrete impls, supporting types) lives next to its
//! domain.

mod config;
mod error;
mod in_memory;
mod prover;
mod strategy;
mod task;
mod traits;

pub use config::{ProverConfig, RetryConfig};
pub use error::{ProverError, ProverResult};
pub use in_memory::{InMemoryReceiptStore, InMemoryTaskStore};
pub use prover::{Prover, ProverBuilder};
pub use task::{TaskRecord, TaskRecordData, TaskResult, TaskStatus};
pub use traits::{ProofSpec, ReceiptHook, ReceiptStore, TaskKey, TaskStore};
pub use zkaleido::{ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram};
