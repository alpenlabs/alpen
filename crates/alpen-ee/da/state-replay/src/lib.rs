//! Replays decoded EE DA blobs into reconstructed execution state.
//!
//! This crate applies the state diff from each replayed DA blob. Snapshot
//! artifacts carry replay state; bytecode preimage completeness is outside this
//! crate's replay contract.
//!
//! Replay is externally stateless: callers provide initial state and own
//! persistence of returned snapshots or state artifacts.

mod error;
mod replay;
mod snapshot;
mod summary;

pub use error::ReplayError;
pub use replay::{replay_da_blobs_from_genesis, replay_da_blobs_from_snapshot};
pub use snapshot::{ReplayStateSnapshot, SNAPSHOT_FORMAT_VERSION};
pub use summary::{AppliedExecBlockRange, ReplaySummary};
