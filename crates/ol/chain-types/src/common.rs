use strata_primitives::{Slot, buf::Buf32};

/// Type aliases for clarity
pub type OLBlockId = Buf32;
pub type L1BlockId = Buf32;

/// Commitment to a block by ID at a particular slot.
#[derive(Clone, Debug)]
pub struct OLBlockCommitment {
    slot: Slot,
    blkid: OLBlockId,
}

impl OLBlockCommitment {
    pub fn new(slot: Slot, blkid: OLBlockId) -> Self {
        Self { slot, blkid }
    }

    pub fn slot(&self) -> Slot {
        self.slot
    }

    pub fn blkid(&self) -> &OLBlockId {
        &self.blkid
    }
}

/// Commitment to an L1 block by ID at a particular height.
/// Useful since Bitcoin blocks do not include height in their header.
#[derive(Clone, Debug)]
pub struct L1BlockCommitment {
    height: u32,
    blkid: L1BlockId,
}

impl L1BlockCommitment {
    pub fn new(height: u32, blkid: L1BlockId) -> Self {
        Self { height, blkid }
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }
}
