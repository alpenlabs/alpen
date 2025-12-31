//! Alpen EVM EE Account Proof Implementation
//!
//! This crate implements the **guest-side** proof generation for Alpen EVM EE account updates.
//! It provides:
//!
//! - **Inner proof (chunk)**: `AlpenChunkProofProgram` - executes EVM blocks in a chunk
//! - **Outer proof (batch)**: `AlpenBatchProofProgram` - aggregates and verifies chunks
//!
//! **Note**: Host-side data fetching logic should be implemented by the application
//! using this crate.

// Guest-side block building
mod guest_builder;

// Inner proof (chunk proof) logic
pub mod inner;

// Outer proof (batch proof) logic
pub mod outer;

// ZkVmProgram implementations
mod batch_program;
mod chunk_program;

// Input types
mod types;

pub use batch_program::{AlpenBatchProofProgram, BatchProofInput, BatchProofProgramOutput};
pub use chunk_program::{AlpenChunkProofProgram, ChunkProofInput, ChunkProofProgramOutput};
pub use types::{ChunkProofOutput, CommitBlockPackage, EeAccountInit};
