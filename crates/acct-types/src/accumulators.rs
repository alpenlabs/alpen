//! Types relating to accumulators and making proofs against them.

use strata_identifiers::{Hash, RBuf32};
use strata_merkle::Mmr;
use tree_hash::TreeHash;

use crate::{
    AccumulatorClaim, L1BlockRecord,
    mmr::{Mmr64, StrataHasher},
};

impl AccumulatorClaim {
    /// Creates a new accumulator claim.
    pub fn new(idx: u64, entry_hash: impl Into<[u8; 32]>) -> Self {
        Self {
            idx,
            entry_hash: Into::<[u8; 32]>::into(entry_hash).into(),
        }
    }

    /// Gets the index.
    pub fn idx(&self) -> u64 {
        self.idx
    }

    /// Gets the entry hash.
    pub fn entry_hash(&self) -> Hash {
        self.entry_hash
            .as_ref()
            .try_into()
            .expect("acct-types: FixedBytes<32> is always 32 bytes")
    }
}

impl L1BlockRecord {
    /// Creates an L1 block record.
    pub fn new(block_hash: impl Into<[u8; 32]>, wtxids_root: impl Into<[u8; 32]>) -> Self {
        Self {
            block_hash: RBuf32(block_hash.into()),
            wtxids_root: RBuf32(wtxids_root.into()),
        }
    }

    /// Gets the referenced Bitcoin block hash.
    pub fn block_hash(&self) -> [u8; 32] {
        self.block_hash.0
    }

    /// Gets the block witness transaction Merkle root.
    pub fn wtxids_root(&self) -> [u8; 32] {
        self.wtxids_root.0
    }

    /// Computes the canonical OL L1 block refs MMR leaf hash.
    pub fn leaf_hash(&self) -> [u8; 32] {
        <L1BlockRecord as TreeHash>::tree_hash_root(self).into_inner()
    }
}

/// Computes the canonical OL L1 block refs MMR leaf hash for the given
/// `{block_hash, wtxids_root}`.
pub fn l1_block_record_leaf_hash(block_hash: &[u8; 32], wtxids_root: &[u8; 32]) -> [u8; 32] {
    L1BlockRecord::new(*block_hash, *wtxids_root).leaf_hash()
}

/// Appends an [`L1BlockRecord`]'s leaf into the OL L1 block refs MMR.
///
/// Centralizes how an [`L1BlockRecord`] becomes an MMR leaf so that the
/// canonical state and the write-batch (diff) path always commit to
/// identical leaves.
pub fn append_l1_block_rec_to_mmr(mmr: &mut Mmr64, rec: &L1BlockRecord) {
    Mmr::<StrataHasher>::add_leaf(mmr, rec.leaf_hash())
        .expect("acct-types: L1 block refs MMR capacity exceeded");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l1_block_record_leaf_hash_matches_pinned_root() {
        let block_hash = [1u8; 32];
        let wtxids_root = [2u8; 32];
        // Captured from `l1_block_record_leaf_hash(&[1u8; 32], &[2u8; 32])` on a
        // known-good run; pinned here so any change to the `L1BlockRecord` SSZ
        // TreeHash layout (field order, types, container shape) trips this test.
        // This is the OL L1-block-refs MMR commitment and must stay byte-stable.
        let expected = [
            248, 24, 175, 211, 122, 109, 195, 188, 146, 251, 68, 115, 16, 17, 39, 112, 6, 219, 78,
            250, 110, 144, 35, 205, 116, 104, 192, 35, 53, 210, 42, 77,
        ];

        assert_eq!(
            l1_block_record_leaf_hash(&block_hash, &wtxids_root),
            expected
        );
    }

    #[test]
    fn l1_block_record_accessors_round_trip() {
        let block_hash = [3u8; 32];
        let wtxids_root = [4u8; 32];
        let record = L1BlockRecord::new(block_hash, wtxids_root);

        assert_eq!(record.block_hash(), block_hash);
        assert_eq!(record.wtxids_root(), wtxids_root);
    }
}
