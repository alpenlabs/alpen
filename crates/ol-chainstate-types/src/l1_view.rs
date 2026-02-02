use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_identifiers::L1BlockCommitment;
use strata_primitives::l1::L1BlockId;

/// Describes state relating to the CL's view of L1.  Updated by entries in the
/// L1 segment of CL blocks.
#[derive(
    Clone,
    Debug,
    Eq,
    PartialEq,
    BorshDeserialize,
    BorshSerialize,
    Arbitrary,
    rkyv::Archive,
    rkyv::Serialize,
    rkyv::Deserialize,
)]
pub struct L1ViewState {
    /// The actual first block we ever looked at.
    pub(crate) genesis_height: u64,

    /// Verified L1Block
    pub(crate) verified_blk: L1BlockCommitment,
}

impl L1ViewState {
    /// Creates a new instance with the genesis trigger L1 block already ingested.
    pub fn new_at_genesis(genesis_blk: L1BlockCommitment) -> Self {
        Self {
            genesis_height: genesis_blk.height_u64(),
            verified_blk: genesis_blk,
        }
    }

    pub fn safe_blkid(&self) -> &L1BlockId {
        self.verified_blk.blkid()
    }

    pub fn safe_height(&self) -> u64 {
        self.verified_blk.height_u64()
    }

    /// Gets the safe block as a [`L1BlockCommitment`].
    pub fn get_safe_block(&self) -> L1BlockCommitment {
        self.verified_blk
    }

    /// The height of the next block we expect to be added.
    pub fn next_expected_height(&self) -> u64 {
        self.safe_height() + 1
    }

    pub fn update_verified_blk(&mut self, verified_blk: L1BlockCommitment) {
        self.verified_blk = verified_blk;
    }
}
