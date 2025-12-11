//! Epoch summary types for checkpoint state management.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, Epoch, EpochCommitment, L1BlockCommitment, L2BlockCommitment};

/// Summary generated when we accept the last block of an epoch.
///
/// It's possible in theory for more than one of these to validly exist for a
/// single epoch, but not in the same chain.
#[derive(
    Copy, Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct EpochSummary {
    /// The epoch number.
    epoch: Epoch,

    /// The last block of the checkpoint.
    terminal: L2BlockCommitment,

    /// The previous epoch that this epoch was built on.
    /// If this is the genesis epoch, then this is all zero.
    prev_terminal: L2BlockCommitment,

    /// The new L1 block that was submitted in the terminal block.
    new_l1: L1BlockCommitment,

    /// The final state root of the epoch.
    final_state: Buf32,
}

impl EpochSummary {
    /// Creates a new instance.
    pub fn new(
        epoch: Epoch,
        terminal: L2BlockCommitment,
        prev_terminal: L2BlockCommitment,
        new_l1: L1BlockCommitment,
        final_state: Buf32,
    ) -> Self {
        Self {
            epoch,
            terminal,
            prev_terminal,
            new_l1,
            final_state,
        }
    }

    /// Returns the epoch number.
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Returns the terminal block commitment.
    pub fn terminal(&self) -> &L2BlockCommitment {
        &self.terminal
    }

    /// Returns the previous terminal block commitment.
    pub fn prev_terminal(&self) -> &L2BlockCommitment {
        &self.prev_terminal
    }

    /// Returns the new L1 block commitment.
    pub fn new_l1(&self) -> &L1BlockCommitment {
        &self.new_l1
    }

    /// Returns the final state root.
    pub fn final_state(&self) -> &Buf32 {
        &self.final_state
    }

    /// Generates an epoch commitment for this epoch.
    pub fn get_epoch_commitment(&self) -> EpochCommitment {
        EpochCommitment::new(self.epoch, self.terminal.slot(), *self.terminal.blkid())
    }

    /// Gets the epoch commitment for the previous epoch.
    pub fn get_prev_epoch_commitment(&self) -> Option<EpochCommitment> {
        if self.epoch == 0 {
            None
        } else {
            Some(EpochCommitment::new(
                self.epoch - 1,
                self.prev_terminal.slot(),
                *self.prev_terminal.blkid(),
            ))
        }
    }

    /// Create the summary for the next epoch based on this one.
    pub fn create_next_epoch_summary(
        &self,
        new_terminal: L2BlockCommitment,
        new_l1: L1BlockCommitment,
        new_state: Buf32,
    ) -> EpochSummary {
        Self::new(
            self.epoch + 1,
            new_terminal,
            self.terminal,
            new_l1,
            new_state,
        )
    }
}
