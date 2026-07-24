//! Reconstructs an EE state and materializes a sparse sequencer datadir.
//!
//! The stopped source database supplies only ordered commit/reveal transaction
//! IDs. Bitcoin supplies the DA transaction contents. A synced OL node supplies
//! proof-backed account state, accepted execution tips, input cursors, batch
//! linkage, and the safe finalized OL restart anchor.
//!
//! Recovery replays the hydrated DA prefix, checks both the execution state root
//! and the full [`strata_ee_acct_types::EeAccountState`] commitment, initializes
//! Reth and the minimum EE Sled records in a sibling staging directory, validates
//! those records, and only then renames the staging directory to the requested
//! output path.

mod bitcoin;
mod ol;
mod reconstruct;
mod recover;
mod reth_import;
mod sled_bootstrap;
mod source;
mod validate;

pub(crate) use recover::ee_state_recover;
pub use recover::EeStateRecoverArgs;
