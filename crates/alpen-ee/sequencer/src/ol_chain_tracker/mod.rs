//! Track OL chain and store inbox messages for finalized blocks for use in block assembly.
mod handle;
mod state;
mod task;

#[cfg(test)]
pub(crate) mod test_utils;

pub use handle::{build_ol_chain_tracker, OLChainTrackerHandle};
pub use state::{init_ol_chain_tracker_state, OLChainTrackerState};
