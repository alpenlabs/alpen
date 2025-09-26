//! Interpretation of extra data.

use strata_acct_types::Hash;

/// Message sent in the extra data field in the update operation.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct UpdateExtraData {
    /// The blkid of the new execution tip block.
    new_tip_blkid: Hash,

    /// The total number of items to remove from the input queue.
    processed_inputs: u32,

    /// The total number of items to remove from the fincl queue.
    processed_fincls: u32,
}

impl UpdateExtraData {
    pub fn new(new_tip_blkid: Hash, processed_inputs: u32, processed_fincls: u32) -> Self {
        Self {
            new_tip_blkid,
            processed_inputs,
            processed_fincls,
        }
    }

    pub fn decode(buf: &[u8]) -> Result<Self, ()> {
        // TODO
        unimplemented!()
    }

    pub fn new_tip_blkid(&self) -> [u8; 32] {
        self.new_tip_blkid
    }

    pub fn processed_inputs(&self) -> u32 {
        self.processed_inputs
    }

    pub fn processed_fincls(&self) -> u32 {
        self.processed_fincls
    }
}
