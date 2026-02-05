use std::fmt;

use arbitrary::Arbitrary;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use strata_identifiers::{
    Buf32, Epoch, EpochCommitment, L1BlockCommitment, L2BlockCommitment, L2BlockId, Slot,
};

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
    Deserialize,
    Serialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
    Encode,
    Decode,
)]
pub struct EpochSummary {
    /// The epoch number.
    ///
    /// These are always sequential.
    epoch: Epoch,

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

    pub fn epoch(&self) -> Epoch {
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
    Clone,
    Debug,
    Eq,
    PartialEq,
    Arbitrary,
    Deserialize,
    Serialize,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct BatchInfo {
    /// Checkpoint epoch
    pub epoch: Epoch,

    /// L1 block range(inclusive) the checkpoint covers
    pub l1_range: (L1BlockCommitment, L1BlockCommitment),

    /// L2 block range(inclusive) the checkpoint covers
    pub l2_range: (L2BlockCommitment, L2BlockCommitment),
}

impl ssz::Encode for BatchInfo {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        <Epoch as ssz::Encode>::ssz_fixed_len()
            + 2 * <L1BlockCommitment as ssz::Encode>::ssz_fixed_len()
            + 2 * <L2BlockCommitment as ssz::Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        self.epoch.ssz_append(buf);
        self.l1_range.0.ssz_append(buf);
        self.l1_range.1.ssz_append(buf);
        self.l2_range.0.ssz_append(buf);
        self.l2_range.1.ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        <Epoch as ssz::Encode>::ssz_fixed_len()
            + 2 * <L1BlockCommitment as ssz::Encode>::ssz_fixed_len()
            + 2 * <L2BlockCommitment as ssz::Encode>::ssz_fixed_len()
    }
}

impl ssz::Decode for BatchInfo {
    fn is_ssz_fixed_len() -> bool {
        true
    }

    fn ssz_fixed_len() -> usize {
        <Epoch as ssz::Encode>::ssz_fixed_len()
            + 2 * <L1BlockCommitment as ssz::Encode>::ssz_fixed_len()
            + 2 * <L2BlockCommitment as ssz::Encode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, ssz::DecodeError> {
        let fixed_len = <Epoch as ssz::Encode>::ssz_fixed_len()
            + 2 * <L1BlockCommitment as ssz::Encode>::ssz_fixed_len()
            + 2 * <L2BlockCommitment as ssz::Encode>::ssz_fixed_len();
        if bytes.len() != fixed_len {
            return Err(ssz::DecodeError::InvalidByteLength {
                len: bytes.len(),
                expected: fixed_len,
            });
        }

        let epoch_len = <Epoch as ssz::Encode>::ssz_fixed_len();
        let l1_len = <L1BlockCommitment as ssz::Encode>::ssz_fixed_len();
        let l2_len = <L2BlockCommitment as ssz::Encode>::ssz_fixed_len();

        let mut offset = 0;
        let epoch = Epoch::from_ssz_bytes(&bytes[offset..offset + epoch_len])?;
        offset += epoch_len;
        let l1_start = L1BlockCommitment::from_ssz_bytes(&bytes[offset..offset + l1_len])?;
        offset += l1_len;
        let l1_end = L1BlockCommitment::from_ssz_bytes(&bytes[offset..offset + l1_len])?;
        offset += l1_len;
        let l2_start = L2BlockCommitment::from_ssz_bytes(&bytes[offset..offset + l2_len])?;
        offset += l2_len;
        let l2_end = L2BlockCommitment::from_ssz_bytes(&bytes[offset..offset + l2_len])?;

        Ok(Self {
            epoch,
            l1_range: (l1_start, l1_end),
            l2_range: (l2_start, l2_end),
        })
    }
}

impl fmt::Display for BatchInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

impl BatchInfo {
    pub fn new(
        checkpoint_idx: Epoch,
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
    pub fn epoch(&self) -> Epoch {
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

    /// Check if the L2 slot is at or before the end of this checkpoint's L2 range.
    ///
    /// Note: This only checks the upper bound, not the lower bound. It returns `true`
    /// if the slot is <= the checkpoint's final L2 slot. Use this to check if a slot
    /// is included in any batch up to this one (including).
    pub fn l2_slot_at_or_before_end(&self, slot: Slot) -> bool {
        let (_, last_l2_commitment) = self.l2_range;
        slot <= last_l2_commitment.slot()
    }

    /// Check if the L1 height is at or before the end of this checkpoint's L1 range.
    ///
    /// Note: This only checks the upper bound, not the lower bound. It returns `true`
    /// if the height is <= the checkpoint's final L1 height.
    pub fn l1_height_at_or_before_end(&self, height: u64) -> bool {
        let (_, last_l1_commitment) = self.l1_range;
        height <= last_l1_commitment.height_u64()
    }
}
