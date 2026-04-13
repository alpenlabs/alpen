//! Single-proof-type proving engine with zkaleido integration.
//!
//! Each [`Prover`] wraps one [`ProofSpec`] and a prove strategy (native or remote).
//! The spec fetches inputs. Receipt storage and domain hooks are opt-in.
//! The prover runs the zkVM program via the strategy.

pub mod config;
pub mod error;
pub mod prover;
pub mod receipt;
pub mod spec;
pub mod store;
pub mod strategy;
pub mod task;

pub use config::{ProverConfig, RetryConfig};
pub use error::{ProverError, ProverResult};
pub use prover::{Prover, ProverBuilder};
pub use receipt::{InMemoryReceiptStore, ReceiptHook, ReceiptStore};
pub use spec::ProofSpec;
#[cfg(feature = "sled")]
pub use store::SledTaskStore;
pub use store::{InMemoryTaskStore, TaskRecord, TaskStore};
pub use strategy::{ProveContext, ProveStrategy};
pub use task::{TaskResult, TaskStatus};
pub use zkaleido::{ProofReceiptWithMetadata, ZkVmHost, ZkVmProgram};
