//! Data availability accumulation for block assembly.
//!
//! This module handles accumulating state diffs and logs across blocks within an epoch.
//! At epoch boundaries, the accumulated data is finalized and reset for the next epoch.

use strata_identifiers::Epoch;
use strata_ol_chain_types_new::OLLog;
use strata_ol_da::StateDiff;

/// Accumulated DA data for a block within an epoch.
///
/// Contains both the state diff accumulated so far and the logs
/// generated up to this point in the epoch.
#[derive(Debug)]
pub struct AccumulatedDaData {
    /// The epoch this accumulation belongs to.
    pub epoch: Epoch,

    /// The accumulated state diff from epoch start to this block.
    /// This is built incrementally as blocks are processed.
    pub state_diff: StateDiff,

    /// All logs emitted in the epoch up to and including this block.
    pub logs: Vec<OLLog>,
}

impl AccumulatedDaData {
    /// Creates empty accumulated data for the start of an epoch.
    pub fn empty(epoch: Epoch) -> Self {
        Self {
            epoch,
            state_diff: StateDiff::default(),
            logs: Vec::new(),
        }
    }

    /// Creates accumulated data with the given components.
    pub fn new(epoch: Epoch, state_diff: StateDiff, logs: Vec<OLLog>) -> Self {
        Self {
            epoch,
            state_diff,
            logs,
        }
    }

    /// Checks if this is the start of a new epoch compared to another.
    pub fn is_new_epoch(&self, other_epoch: Epoch) -> bool {
        self.epoch != other_epoch
    }

    /// Appends logs to the accumulated logs.
    pub fn append_logs(&mut self, new_logs: Vec<OLLog>) {
        self.logs.extend(new_logs);
    }
}