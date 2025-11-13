//! Registry-based proof generation system
//!
//! The registry pattern enables dynamic handler registration for different program types,
//! providing type-safe extensibility without exposing implementation details.
//!
//! ## Architecture
//!
//! - **Core Types** (`core`): `ProgramType`, `ProgramHandler`, `ProgramRegistry`
//! - **Builder** (`builder`): Fluent API for registering handlers
//! - **Handle** (`handle`): Public API for submitting tasks and querying status
//! - **Prover** (`prover`): Prover implementation that uses the registry
//!
//! ## Example
//!
//! ```rust,ignore
//! use strata_paas::registry::RegistryProverServiceBuilder;
//!
//! let handle = RegistryProverServiceBuilder::new(config)
//!     .register::<MyProgram, _, _, _>(
//!         MyVariant::A,
//!         input_fetcher,
//!         proof_store,
//!         host,
//!     )
//!     .launch(&executor)
//!     .await?;
//! ```

mod builder;
mod core;
mod handle;
mod prover;

// Re-export public API types
pub use builder::RegistryProverServiceBuilder;
pub use core::{
    BoxedInput, BoxedProof, ConcreteHandler, InputFetcher, ProgramHandler, ProgramRegistry,
    ProgramType, ProofStore,
};
pub use handle::RegistryProverHandle;
pub use prover::RegistryProver;
