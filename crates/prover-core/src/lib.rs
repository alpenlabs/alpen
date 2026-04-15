//! Single-proof-type proving engine with zkaleido integration.
//!
//! Each [`Prover`] wraps one [`ProofSpec`] and a prove strategy (native or remote).
//! The spec fetches inputs. Receipt storage and domain hooks are opt-in.
//! The prover runs the zkVM program via the strategy.

mod config;
mod error;
mod prover;
mod receipt;
mod spec;
mod store;
mod strategy;
mod task;

pub use config::{ProverConfig, RetryConfig};
pub use error::{ProverError, ProverResult};
pub use prover::{Prover, ProverBuilder};
pub use receipt::{InMemoryReceiptStore, ReceiptHook, ReceiptStore};
pub use spec::{ProofSpec, TaskKey};
pub use store::{
    now_secs, InMemoryTaskStore, SecsSinceEpoch, TaskRecord, TaskRecordData, TaskStore,
};
pub use strategy::{ProveContext, ProveStrategy};
pub use task::{TaskResult, TaskStatus};
pub use zkaleido::{ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram};
