use std::fmt;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, EpochCommitment, L1BlockCommitment, L2BlockCommitment, L2BlockId};

/// Summary generated when we accept the last block of an epoch.
///
/// It's possible in theory for more than one of these to validly exist for a
/// single epoch, but not in the same chain.
#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Arbitrary,
    BorshDeserialize,
    BorshSerialize,
    Deserialize,
    Serialize,
)]
pub struct EpochSummary {
    /// The epoch number.
    ///
    /// These are always sequential.
    epoch: u64,

    /// The last block of the checkpoint.
    terminal: L2BlockCommitment,

    /// The previous epoch that this epoch was built on.
    ///
    /// If this is the genesis epoch, then this is all zero.
    prev_terminal: L2BlockCommitment,

    /// The new L1 block that was submitted in the terminal block.
    new_l1: L1BlockCommitment,

    /// The final state root of the epoch.
    ///
    /// Currently this is just copied from the state root of the header of the
    /// last block of the slot, but it's likely we'll change this to add
    /// processing outside of the terminal block before "finishing" the epoch.
    final_state: Buf32,
}

impl EpochSummary {
    /// Creates a new instance.
    pub fn new(
        epoch: u64,
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

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn terminal(&self) -> &L2BlockCommitment {
        &self.terminal
    }

    pub fn prev_terminal(&self) -> &L2BlockCommitment {
        &self.prev_terminal
    }

    pub fn new_l1(&self) -> &L1BlockCommitment {
        &self.new_l1
    }

    pub fn final_state(&self) -> &Buf32 {
        &self.final_state
    }

    /// Generates an epoch commitent for this epoch using the data in the
    /// summary.
    pub fn get_epoch_commitment(&self) -> EpochCommitment {
        EpochCommitment::new(self.epoch, self.terminal.slot(), *self.terminal.blkid())
    }

    /// Gets the epoch commitment for the previous epoch, using the terminal
    /// block reference the header stores.
    pub fn get_prev_epoch_commitment(self) -> Option<EpochCommitment> {
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
            self.epoch() + 1,
            new_terminal,
            *self.terminal(),
            new_l1,
            new_state,
        )
    }
}

/// Contains metadata describing a batch checkpoint, including the L1 and L2 height ranges
/// it covers and the final L2 block ID in that range.
#[derive(
    Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize, Deserialize, Serialize,
)]
pub struct BatchInfo {
    /// Checkpoint epoch
    pub epoch: u64,

    /// L1 block range(inclusive) the checkpoint covers
    pub l1_range: (L1BlockCommitment, L1BlockCommitment),

    /// L2 block range(inclusive) the checkpoint covers
    pub l2_range: (L2BlockCommitment, L2BlockCommitment),
}

impl fmt::Display for BatchInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

impl BatchInfo {
    pub fn new(
        checkpoint_idx: u64,
        l1_range: (L1BlockCommitment, L1BlockCommitment),
        l2_range: (L2BlockCommitment, L2BlockCommitment),
    ) -> Self {
        Self {
            epoch: checkpoint_idx,
            l1_range,
            l2_range,
        }
    }

    /// Geets the epoch index.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Gets the epoch commitment for this batch.
    pub fn get_epoch_commitment(&self) -> EpochCommitment {
        EpochCommitment::from_terminal(self.epoch(), *self.final_l2_block())
    }

    /// Gets the final L2 block commitment in the batch's L2 range.
    pub fn final_l2_block(&self) -> &L2BlockCommitment {
        &self.l2_range.1
    }

    /// Gets the final L2 blkid in the batch's L2 range.
    pub fn final_l2_blockid(&self) -> &L2BlockId {
        self.l2_range.1.blkid()
    }

    /// Gets the final L1 block commitment in the batch's L1 range.
    pub fn final_l1_block(&self) -> &L1BlockCommitment {
        &self.l1_range.1
    }

    /// Check is whether the L2 slot is covered by the checkpoint
    pub fn includes_l2_block(&self, slot: u64) -> bool {
        let (_, last_l2_commitment) = self.l2_range;
        if slot <= last_l2_commitment.slot() {
            return true;
        }
        false
    }

    /// check for whether the L1 height is covered by the checkpoint
    pub fn includes_l1_block(&self, height: u64) -> bool {
        let (_, last_l1_commitment) = self.l1_range;
        if height <= last_l1_commitment.height_u64() {
            return true;
        }
        false
    }
}
