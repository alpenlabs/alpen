//! Replays decoded EE DA blobs into reconstructed execution state.
//!
//! This crate applies the bytecodes present in each replayed DA blob. It does
//! not prove that an omitted bytecode preimage was previously published on L1;
//! consumers that need trustless bytecode availability must track preimages
//! across replay windows.

mod error;
mod replay;
mod snapshot;
mod summary;

pub use error::ReplayError;
pub use replay::{replay_blobs_from_genesis, replay_blobs_from_snapshot};
pub use snapshot::ReplayPreStateSnapshot;
pub use summary::{AppliedExecBlockRange, ReplaySummary};
