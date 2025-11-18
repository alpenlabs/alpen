//! Types for ASM manifest and logging.
//!
//! This crate contains the core types used for ASM manifests and log entries,
//! separated from the main ASM common crate to avoid circular dependencies.

mod errors;
mod log;
mod manifest;

pub use errors::*;
pub use log::*;
pub use manifest::*;

/// Type alias for a 32-byte hash.
// TODO use Buf32 from identifiers?
pub type Hash32 = [u8; 32];
