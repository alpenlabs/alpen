//! Impl blocks for checkpoint state types.

use strata_identifiers::{Buf32, Epoch, EpochCommitment, OLBlockCommitment};

use crate::{L1BlockCommitment, ssz_generated::ssz::state::EpochSummary};

impl EpochSummary {
    pub fn new(
        epoch: Epoch,
        terminal: OLBlockCommitment,
        prev_terminal: OLBlockCommitment,
        l1_end: L1BlockCommitment,
        final_state: Buf32,
    ) -> Self {
        Self {
            epoch,
            terminal,
            prev_terminal,
            l1_end,
            final_state,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn terminal(&self) -> &OLBlockCommitment {
        &self.terminal
    }

    pub fn prev_terminal(&self) -> &OLBlockCommitment {
        &self.prev_terminal
    }

    pub fn l1_end(&self) -> &L1BlockCommitment {
        &self.l1_end
    }

    pub fn final_state(&self) -> &Buf32 {
        &self.final_state
    }

    /// Check if this is the genesis epoch (prev_terminal blkid is zero).
    pub fn is_genesis(&self) -> bool {
        Buf32::from(*self.prev_terminal.blkid()).is_zero()
    }

    /// Get the epoch commitment for this epoch.
    pub fn get_epoch_commitment(&self) -> EpochCommitment {
        EpochCommitment::from_terminal(self.epoch, self.terminal)
    }

    /// Get the epoch commitment for the previous epoch.
    /// Returns `None` if this is the genesis epoch.
    pub fn get_prev_epoch_commitment(&self) -> Option<EpochCommitment> {
        if self.epoch == 0 {
            return None;
        }
        Some(EpochCommitment::from_terminal(
            self.epoch - 1,
            self.prev_terminal,
        ))
    }

    /// Create the summary for the next epoch based on this one.
    pub fn create_next_epoch_summary(
        &self,
        new_terminal: OLBlockCommitment,
        l1_end: L1BlockCommitment,
        new_state: Buf32,
    ) -> EpochSummary {
        EpochSummary::new(
            self.epoch + 1,
            new_terminal,
            self.terminal,
            l1_end,
            new_state,
        )
    }
}
