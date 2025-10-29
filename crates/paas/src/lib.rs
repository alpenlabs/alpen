//! Prover-as-a-Service (PaaS) Library
//!
//! This crate provides an embeddable proof generation service that follows
//! Strata's command worker pattern. It can be embedded in binaries like
//! `strata-client` or exposed via an optional REST API wrapper.
//!
//! # Architecture
//!
//! PaaS follows the command worker pattern from `crates/service`:
//! - **ProverService**: AsyncService implementation
//! - **ProverHandle**: Command-based API for interaction
//! - **ProverBuilder**: Fluent API for construction
//!
//! # Example
//!
//! ```no_run
//! use strata_paas::{PaaSConfig, ProverBuilder};
//! use strata_primitives::proof::ProofContext;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let executor = strata_tasks::TaskExecutor::new();
//!
//! // Build and launch prover service
//! let handle = ProverBuilder::new()
//!     .with_config(PaaSConfig::default())
//!     .with_proof_operator(proof_operator)
//!     .with_database(database)
//!     .launch(&executor)?;
//!
//! // Submit proof request
//! let task_id = handle
//!     .create_task(
//!         ProofContext::Checkpoint { index: 42 },
//!         vec![], // no dependencies
//!     )
//!     .await?;
//!
//! // Poll for completion
//! let status = handle.get_task_status(task_id).await?;
//! # Ok(())
//! # }
//! ```

// Public API exports
pub use builder::ProverBuilder;
pub use commands::{PaaSCommand, TaskId};
pub use config::PaaSConfig;
pub use errors::PaaSError;
pub use handle::ProverHandle;
pub use service::ProverService;
pub use status::{PaaSReport, PaaSStatus, TaskStatus};

// Internal modules
mod builder;
mod commands;
mod config;
mod errors;
mod handle;
mod service;
mod state;
mod status;

// Public modules
pub mod manager;

// Re-export key manager types
// Re-export config types
pub use config::{FeatureConfig, RetryConfig, WorkerConfig};
pub use manager::{ProofOperatorTrait, TaskTracker, WorkerPool};
