//! Types for ASM manifest and logging.
//!
//! This crate contains the core types used for ASM manifests and log entries,
//! separated from the main ASM common crate to avoid circular dependencies.

mod errors;
mod log;
mod manifest;
mod payloads;

// Include generated SSZ types
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use errors::*;
pub use log::*;
pub use payloads::*;
// Re-export generated SSZ types
pub use ssz_generated::ssz::{
    self as ssz,
    log::{AsmLogEntry, AsmLogEntryRef},
    manifest::{AsmManifest, AsmManifestRef},
};

/// Type alias for a 32-byte hash.
// TODO use Buf32 from identifiers?
pub type Hash32 = [u8; 32];
