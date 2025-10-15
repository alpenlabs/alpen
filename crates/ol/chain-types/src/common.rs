use strata_primitives::buf::Buf32;

/// Type aliases for clarity
pub type Slot = u64;
pub type Epoch = u32;
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

/// Commitment to the terminal block of a particular epoch.
#[derive(Clone, Debug)]
pub struct EpochCommitment {
    epoch: Epoch,
    terminal_block: OLBlockCommitment,
}

impl EpochCommitment {
    pub fn new(epoch: Epoch, terminal_block: OLBlockCommitment) -> Self {
        Self {
            epoch,
            terminal_block,
        }
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn terminal_block(&self) -> &OLBlockCommitment {
        &self.terminal_block
    }

    pub fn terminal_slot(&self) -> Slot {
        self.terminal_block.slot()
    }

    pub fn terminal_blkid(&self) -> &OLBlockId {
        self.terminal_block.blkid()
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
