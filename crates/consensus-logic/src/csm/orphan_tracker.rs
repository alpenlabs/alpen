use std::collections::HashSet;

use strata_state::l1::L1BlockId;

use super::common::{IndexedBlockTable, L1Header};

#[derive(Debug, Default)]
pub struct OrphanTracker {
    pub(super) blocks: IndexedBlockTable,
    pub(super) heads: HashSet<L1BlockId>,
}

impl OrphanTracker {
    pub fn insert(&mut self, block: L1Header) {
        if self.blocks.by_block_id.contains_key(&block.block_id()) {
            // duplicate
            return;
        }

        self.blocks.insert(block);

        if !self.blocks.by_block_id.contains_key(&block.parent_id()) {
            // extends an existing chain
            self.heads.insert(block.block_id());
        }

        if let Some(heads) = self.blocks.by_parent_id.get(&block.block_id()) {
            // this block is a head ahead of existing blocks
            for block_id in heads {
                self.heads.remove(block_id);
            }
        }
    }

    pub fn remove(&mut self, block_id: &L1BlockId) {
        // remove the block from the internal table
        if self.blocks.remove(block_id).is_none() {
            // block doesn't exist, there's nothing more to do.
            return;
        }

        // block is no longer a head, if it was one.
        self.heads.remove(block_id);

        // children of the removed block,if they exist, become new heads
        if let Some(children_ids) = self.blocks.by_parent_id.get(block_id) {
            for child_id in children_ids {
                self.heads.insert(*child_id);
            }
        }
    }

    pub fn children(&self, block_id: &L1BlockId) -> Option<&Vec<L1BlockId>> {
        self.blocks.by_parent_id.get(block_id)
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::{block::Version, hashes::Hash, BlockHash, TxMerkleNode};
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::csm::common::U256;

    fn u64_to_u83_32(n: u64) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        bytes[0..8].copy_from_slice(&n.to_be_bytes());
        bytes
    }

    fn make_l1_header(
        height: u64,
        parent_id: u64,
        block_id: u64,
        accumulated_pow: u64,
    ) -> L1Header {
        // let block_id = L1BlockId::from_u64(block_id);

        L1Header::from_parts(
            height,
            Buf32(u64_to_u83_32(block_id)).into(),
            U256::from_be_bytes(u64_to_u83_32(accumulated_pow)),
            bitcoin::block::Header {
                version: Version::TWO,
                prev_blockhash: BlockHash::from_byte_array(u64_to_u83_32(parent_id)),
                merkle_root: TxMerkleNode::all_zeros(),
                time: Default::default(),
                bits: Default::default(),
                nonce: Default::default(),
            },
        )
    }

    #[test]
    fn test_orphan_tracker_insert() {
        let mut orphan_tracker = OrphanTracker::default();

        let blocks = [
            make_l1_header(5, 0x40, 0x50, 0),
            make_l1_header(6, 0x50, 0x60, 0),
            make_l1_header(2, 0x10, 0x20, 0),
            make_l1_header(5, 0x40, 0x51, 0),
            make_l1_header(4, 0x30, 0x40, 0),
            make_l1_header(3, 0x20, 0x30, 0),
        ];

        // empty tracker
        orphan_tracker.insert(blocks[0]);
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[0].block_id()]),
            "insert on empty tracker"
        );

        orphan_tracker.insert(blocks[1]);
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[0].block_id()]),
            "insert child of existing block"
        );

        orphan_tracker.insert(blocks[2]);
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[0].block_id(), blocks[2].block_id()]),
            "insert unconnected block"
        );

        orphan_tracker.insert(blocks[3]);
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([
                blocks[0].block_id(),
                blocks[2].block_id(),
                blocks[3].block_id()
            ]),
            "insert unconnected block at same height"
        );

        orphan_tracker.insert(blocks[4]);
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[2].block_id(), blocks[4].block_id()]),
            "insert common parent"
        );

        // duplicate
        orphan_tracker.insert(blocks[4]);
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[2].block_id(), blocks[4].block_id()]),
            "insert duplicate"
        );

        orphan_tracker.insert(blocks[5]);
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[2].block_id()]),
            "insert connecting orphan"
        );
    }

    #[test]
    fn test_orphan_tracker_remove() {
        let mut orphan_tracker = OrphanTracker::default();

        let blocks = [
            make_l1_header(5, 0x40, 0x50, 0),
            make_l1_header(6, 0x50, 0x60, 0),
            make_l1_header(2, 0x10, 0x20, 0),
            make_l1_header(5, 0x40, 0x51, 0),
            make_l1_header(4, 0x30, 0x40, 0),
            make_l1_header(3, 0x20, 0x30, 0),
        ];

        for block in &blocks {
            orphan_tracker.insert(*block);
        }

        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[2].block_id()]),
            "initial state"
        );
        assert_eq!(orphan_tracker.blocks.by_block_id.len(), 6, "initial count");

        orphan_tracker.remove(&blocks[5].block_id());
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[2].block_id(), blocks[4].block_id()]),
            "remove connecting block"
        );

        orphan_tracker.remove(&blocks[5].block_id());
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[2].block_id(), blocks[4].block_id()]),
            "remove same block multiple times should have same result"
        );

        orphan_tracker.remove(&Buf32(u64_to_u83_32(0x99999)).into());
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[2].block_id(), blocks[4].block_id()]),
            "remove unknown block"
        );

        orphan_tracker.remove(&blocks[4].block_id());
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([
                blocks[0].block_id(),
                blocks[2].block_id(),
                blocks[3].block_id()
            ]),
            "remove common parent"
        );

        orphan_tracker.remove(&blocks[3].block_id());
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[0].block_id(), blocks[2].block_id()]),
            "remove unconnected block at same height"
        );

        orphan_tracker.remove(&blocks[2].block_id());
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[0].block_id()]),
            "remove unconnected block"
        );

        orphan_tracker.remove(&blocks[1].block_id());
        assert_eq!(
            orphan_tracker.heads,
            HashSet::from([blocks[0].block_id()]),
            "remove child of existing block"
        );

        orphan_tracker.remove(&blocks[0].block_id());
        assert_eq!(orphan_tracker.heads.len(), 0, "empty tracker set");
        assert_eq!(
            orphan_tracker.blocks.by_block_id.len(),
            0,
            "empty tracker table"
        );
    }
}
