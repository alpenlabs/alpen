//! Tracker for keeping track of the tree of unfinalized blocks.

use std::collections::*;

use strata_db::traits::BlockStatus;
use strata_primitives::buf::Buf32;
use strata_state::prelude::*;
use strata_storage::L2BlockManager;
use tracing::warn;

use crate::errors::ChainTipError;

/// Entry in block tracker table we use to relate a block with its immediate
/// relatives.
struct BlockEntry {
    parent: L2BlockId,
    children: HashSet<L2BlockId>,
}

/// Tracks the unfinalized block tree on top of the finalized tip.
pub struct UnfinalizedBlockTracker {
    /// Block that we treat as a base that all of the other blocks that we're
    /// considering uses.
    finalized_tip: L2BlockId,

    /// Table of pending blocks near the tip of the block tree.
    pending_table: HashMap<L2BlockId, BlockEntry>,

    /// Unfinalized chain tips.  This also includes the finalized tip if there's
    /// no pending blocks.
    unfinalized_tips: HashSet<L2BlockId>,
}

impl UnfinalizedBlockTracker {
    /// Creates a new tracker with just a finalized tip and no pending blocks.
    pub fn new_empty(finalized_tip: L2BlockId) -> Self {
        let mut pending_tbl = HashMap::new();
        pending_tbl.insert(
            finalized_tip,
            BlockEntry {
                parent: L2BlockId::from(Buf32::zero()),
                children: HashSet::new(),
            },
        );

        let mut unf_tips = HashSet::new();
        unf_tips.insert(finalized_tip);
        Self {
            finalized_tip,
            pending_table: pending_tbl,
            unfinalized_tips: unf_tips,
        }
    }

    /// Returns the "finalized tip", which is the base of the unfinalized tree.
    pub fn finalized_tip(&self) -> &L2BlockId {
        &self.finalized_tip
    }

    /// Returns `true` if the block is either the finalized tip or is already
    /// known to the tracker.
    pub fn is_seen_block(&self, id: &L2BlockId) -> bool {
        self.finalized_tip == *id || self.pending_table.contains_key(id)
    }

    /// Gets the parent of a block from within the tree.  Returns `None` if the
    /// block or its parent isn't in the tree.  Returns `None` for the finalized
    /// tip block, since its parent isn't in the tree.
    pub fn get_parent(&self, id: &L2BlockId) -> Option<&L2BlockId> {
        if *id == self.finalized_tip {
            return None;
        }
        self.pending_table.get(id).map(|ent| &ent.parent)
    }

    /// Returns an iterator over the chain tips.
    pub fn chain_tips_iter(&self) -> impl Iterator<Item = &L2BlockId> {
        self.unfinalized_tips.iter()
    }

    /// Checks if the block is traceable all the way back to the finalized tip.
    fn sanity_check_parent_seq(&self, blkid: &L2BlockId) -> bool {
        if *blkid == self.finalized_tip {
            return true;
        }

        if let Some(ent) = self.pending_table.get(blkid) {
            self.sanity_check_parent_seq(&ent.parent)
        } else {
            false
        }
    }

    /// Tries to attach a block to the tree.  Does not verify the header
    /// corresponds to the given blockid.
    ///
    /// Returns if this new block forks off and creates a new unfinalized tip
    /// block.
    // TODO do a `SealedL2BlockHeader` thing that includes the blkid
    pub fn attach_block(
        &mut self,
        blkid: L2BlockId,
        header: &SignedL2BlockHeader,
    ) -> Result<bool, ChainTipError> {
        if self.pending_table.contains_key(&blkid) {
            warn!(blkid = ?blkid, "block already attached");
            return Ok(false);
        }

        let parent_blkid = header.parent();

        if let Some(parent_ent) = self.pending_table.get_mut(parent_blkid) {
            parent_ent.children.insert(blkid);
        } else {
            return Err(ChainTipError::AttachMissingParent(blkid, *header.parent()));
        }

        let ent = BlockEntry {
            parent: *header.parent(),
            children: HashSet::new(),
        };

        self.pending_table.insert(blkid, ent);

        // Also update the tips table, removing the parent if it's there.
        let did_replace = self.unfinalized_tips.remove(parent_blkid);
        self.unfinalized_tips.insert(blkid);

        Ok(!did_replace)
    }

    /// Updates the finalized block tip, returning a report that includes the
    /// precise blocks that were finalized transatively and any blocks on
    /// competing chains that were rejected.
    pub fn update_finalized_tip(
        &mut self,
        blkid: &L2BlockId,
    ) -> Result<FinalizeReport, ChainTipError> {
        // Sanity check the block so we know it's here.
        if !self.sanity_check_parent_seq(blkid) {
            return Err(ChainTipError::MissingBlock(*blkid));
        }

        if blkid == &self.finalized_tip {
            // finalizing same tip, nothing to do
            return Ok(FinalizeReport {
                prev_tip: self.finalized_tip,
                finalized: vec![],
                rejected: vec![],
            });
        }

        let mut finalized = vec![];
        let mut at = *blkid;

        // Walk down to the current finalized tip and put everything as finalized.
        while at != self.finalized_tip {
            finalized.push(at);

            // Get the parent of the block we're at
            let ent = self.pending_table.get(&at).unwrap();
            at = ent.parent;
        }

        let mut to_evict = vec![];

        // Walk down from the parent of blkid and find the chains that needs to be evicted
        let mut at = self.pending_table.get(blkid).unwrap().parent;
        loop {
            let ent = self.pending_table.get(&at).unwrap();
            for child in &ent.children {
                if !finalized.contains(child) {
                    to_evict.push(*child);
                }
            }
            if at == self.finalized_tip {
                break;
            }
            at = ent.parent;
        }

        // Put all the blocks of the chains that needs to be evicted
        let mut evicted = to_evict.clone();
        for b in to_evict {
            evicted.extend(self.get_all_descendants(&b))
        }

        // Remove the evicted blocks from the pending table
        for b in &evicted {
            self.remove(b);
        }

        // And also remove blocks that we're finalizing, *except* the new
        // finalized tip.
        for b in &finalized {
            if b != blkid {
                self.remove(b);
            }
        }

        // Just update the finalized tip now.
        let old_tip = self.finalized_tip;
        self.finalized_tip = *blkid;

        // Sanity check and construct the report.
        assert!(
            !finalized.is_empty(),
            "chaintip: somehow finalized no blocks"
        );
        Ok(FinalizeReport {
            prev_tip: old_tip,
            finalized,
            rejected: evicted,
        })
    }

    pub fn get_all_descendants(&self, blkid: &L2BlockId) -> HashSet<L2BlockId> {
        let mut descendants = HashSet::new();
        let mut to_visit = vec![*blkid];

        while let Some(curr_blk) = to_visit.pop() {
            if let Some(entry) = self.pending_table.get(&curr_blk) {
                for child in &entry.children {
                    descendants.insert(*child);
                    to_visit.push(*child);
                }
            }
        }
        descendants
    }

    pub fn remove(&mut self, blkid: &L2BlockId) {
        // First, find and store the parent
        let parent = self.get_parent(blkid).cloned();

        // Remove the block from the pending table
        self.pending_table.remove(blkid);

        // Remove the block from its parent's children list
        if let Some(parent) = parent {
            if let Some(parent_entry) = self.pending_table.get_mut(&parent) {
                parent_entry.children.remove(blkid);
            }
        }

        // Remove the block from unfinalized tips
        self.unfinalized_tips.remove(blkid);
    }

    /// Loads the unfinalized blocks into the tracker which are already in the DB
    pub fn load_unfinalized_blocks(
        &mut self,
        finalized_height: u64,
        chain_tip_height: u64,
        l2_block_manager: &L2BlockManager,
    ) -> anyhow::Result<()> {
        for height in (finalized_height + 1)..=chain_tip_height {
            let Ok(block_ids) = l2_block_manager.get_blocks_at_height_blocking(height) else {
                return Err(anyhow::anyhow!("failed to get blocks at height {}", height));
            };
            let block_ids = block_ids
                .into_iter()
                .filter(|block_id| {
                    let Ok(Some(block_status)) =
                        l2_block_manager.get_block_status_blocking(block_id)
                    else {
                        // missing block status
                        warn!(block_id = ?block_id, "missing block status");
                        return false;
                    };
                    block_status == BlockStatus::Valid
                })
                .collect::<Vec<_>>();

            if block_ids.is_empty() {
                return Err(anyhow::anyhow!("missing blocks at height {}", height));
            }

            for block_id in block_ids {
                if let Some(block) = l2_block_manager.get_block_blocking(&block_id)? {
                    let header = block.header();
                    let _ = self.attach_block(block_id, header);
                }
            }
        }

        Ok(())
    }

    #[cfg(test)]
    pub fn unchecked_set_finalized_tip(&mut self, id: L2BlockId) {
        self.finalized_tip = id;
    }

    #[cfg(test)]
    pub fn insert_fake_block(&mut self, id: L2BlockId, parent: L2BlockId) {
        let ent = BlockEntry {
            parent,
            children: HashSet::new(),
        };

        self.pending_table.insert(id, ent);
    }
}

/// Report of blocks that we finalized when finalizing a new tip and blocks that
/// we've permanently rejected.
#[derive(Clone, Debug)]
pub struct FinalizeReport {
    /// Previous tip.
    prev_tip: L2BlockId,

    /// Block we've newly finalized.  The first one of this
    finalized: Vec<L2BlockId>,

    /// Any blocks that were on competing chains than the one we finalized.
    rejected: Vec<L2BlockId>,
}

impl FinalizeReport {
    /// Returns the blkid that was the previously finalized tip.  It's still
    /// finalized, but there's newer blocks that are also finalized now.
    pub fn prev_tip(&self) -> &L2BlockId {
        &self.prev_tip
    }

    /// The new chain tip that's finalized now.
    pub fn new_tip(&self) -> &L2BlockId {
        if self.finalized.is_empty() {
            &self.prev_tip
        } else {
            &self.finalized[0]
        }
    }

    /// Returns a slice of the blkids that were rejected.
    pub fn rejected(&self) -> &[L2BlockId] {
        &self.rejected
    }

    /// Returns an iterator over the blkids that were rejected.
    pub fn rejected_iter(&self) -> impl Iterator<Item = &L2BlockId> {
        self.rejected.iter()
    }
}

// TODO unit tests

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use strata_db::traits::{BlockStatus, Database, L2DataStore};
    use strata_rocksdb::test_utils::get_common_db;
    use strata_state::{header::L2Header, id::L2BlockId};
    use strata_storage::L2BlockManager;
    use strata_test_utils::l2::gen_l2_chain;

    use crate::unfinalized_tracker;

    fn setup_test_chain(l2_prov: &impl L2DataStore) -> [L2BlockId; 7] {
        // Chain A: g -> a1 -> a2 -> a3
        // Chain B: g -> a1 -> b2 -> b3
        // Chain C: g -> c1

        // a3   b3
        // |     |
        // |     |
        // a2   b2
        // |   /
        // | /
        // a1  c1
        // |  /
        // g
        // |

        let a_chain = gen_l2_chain(None, 3);
        let b_chain = gen_l2_chain(Some(a_chain[1].header().clone()), 2);
        let c_chain = gen_l2_chain(Some(a_chain[0].header().clone()), 1);

        for b in a_chain
            .clone()
            .into_iter()
            .chain(b_chain.clone())
            .chain(c_chain.clone())
        {
            let blockid = b.header().get_blockid();
            l2_prov.put_block_data(b).unwrap();
            l2_prov
                .set_block_status(blockid, BlockStatus::Valid)
                .unwrap();
        }

        [
            a_chain[0].header().get_blockid(),
            a_chain[1].header().get_blockid(),
            c_chain[0].header().get_blockid(),
            a_chain[2].header().get_blockid(),
            b_chain[0].header().get_blockid(),
            a_chain[3].header().get_blockid(),
            b_chain[1].header().get_blockid(),
        ]
    }

    fn check_update_finalized(
        prev_finalized_tip: L2BlockId,
        new_finalized_tip: L2BlockId,
        finalized_blocks: &[L2BlockId],
        rejected_blocks: &[L2BlockId],
        unfinalized_tips: HashSet<L2BlockId>,
        l2_blkman: &L2BlockManager,
    ) {
        // Init the chain tracker from the state we figured out.
        let mut chain_tracker =
            unfinalized_tracker::UnfinalizedBlockTracker::new_empty(prev_finalized_tip);

        chain_tracker
            .load_unfinalized_blocks(0, 3, l2_blkman)
            .unwrap();

        let report = chain_tracker
            .update_finalized_tip(&new_finalized_tip)
            .unwrap();

        assert_eq!(report.prev_tip(), &prev_finalized_tip);
        assert_eq!(report.finalized, finalized_blocks);
        assert_eq!(report.rejected(), rejected_blocks);
        assert_eq!(chain_tracker.finalized_tip, new_finalized_tip);
        assert_eq!(chain_tracker.unfinalized_tips, unfinalized_tips);
    }

    #[test]
    fn test_load_unfinalized_blocks() {
        let db = get_common_db();
        let l2_prov = db.l2_provider();

        let [g, a1, c1, a2, b2, a3, b3] = setup_test_chain(l2_prov.as_ref());

        // Init the chain tracker from the state we figured out.
        let mut chain_tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(g);

        let pool = threadpool::ThreadPool::new(1);
        let blkman = L2BlockManager::new(pool, db);

        chain_tracker
            .load_unfinalized_blocks(0, 3, &blkman)
            .unwrap();

        assert_eq!(chain_tracker.get_parent(&g), None);
        assert_eq!(chain_tracker.get_parent(&a1), Some(&g));
        assert_eq!(chain_tracker.get_parent(&c1), Some(&g));
        assert_eq!(chain_tracker.get_parent(&a2), Some(&a1));
        assert_eq!(chain_tracker.get_parent(&b2), Some(&a1));
        assert_eq!(chain_tracker.get_parent(&a3), Some(&a2));
        assert_eq!(chain_tracker.get_parent(&b3), Some(&b2));

        assert_eq!(
            chain_tracker.pending_table.get(&g).unwrap().children,
            HashSet::from_iter(vec![a1, c1])
        );
        assert_eq!(
            chain_tracker.pending_table.get(&a1).unwrap().children,
            HashSet::from_iter(vec![a2, b2])
        );
        assert_eq!(
            chain_tracker.pending_table.get(&c1).unwrap().children,
            HashSet::from_iter(vec![])
        );
        assert_eq!(
            chain_tracker.pending_table.get(&a2).unwrap().children,
            HashSet::from_iter(vec![a3])
        );
        assert_eq!(
            chain_tracker.pending_table.get(&b2).unwrap().children,
            HashSet::from_iter(vec![b3])
        );
    }

    #[test]
    fn test_get_descendants() {
        let db = get_common_db();
        let l2_prov = db.l2_provider();

        let [g, a1, c1, a2, b2, a3, b3] = setup_test_chain(l2_prov.as_ref());

        // Init the chain tracker from the state we figured out.
        let mut chain_tracker = unfinalized_tracker::UnfinalizedBlockTracker::new_empty(g);

        let pool = threadpool::ThreadPool::new(1);
        let blkman = L2BlockManager::new(pool, db);

        chain_tracker
            .load_unfinalized_blocks(0, 3, &blkman)
            .unwrap();

        assert_eq!(
            chain_tracker.get_all_descendants(&g),
            HashSet::from_iter([a1, c1, a2, b2, a3, b3])
        );
        assert_eq!(
            chain_tracker.get_all_descendants(&a1),
            HashSet::from_iter([a2, a3, b2, b3])
        );
        assert_eq!(chain_tracker.get_all_descendants(&c1).len(), 0);
        assert_eq!(
            chain_tracker.get_all_descendants(&a2),
            HashSet::from_iter([a3])
        );
        assert_eq!(
            chain_tracker.get_all_descendants(&b2),
            HashSet::from_iter([b3])
        );
        assert_eq!(chain_tracker.get_all_descendants(&a3).len(), 0);
        assert_eq!(chain_tracker.get_all_descendants(&b3).len(), 0);
    }

    #[test]
    fn test_update_finalized_tip() {
        let db = get_common_db();
        let l2_prov = db.l2_provider();

        let [g, a1, c1, a2, b2, a3, b3] = setup_test_chain(l2_prov.as_ref());

        eprintln!("{} {} {} {} {} {} {}", g, a1, c1, a2, b2, a3, b3);

        let pool = threadpool::ThreadPool::new(1);
        let blk_manager = L2BlockManager::new(pool, db);

        check_update_finalized(
            g,
            b2,
            &[b2, a1],
            &[a2, c1, a3],
            HashSet::from_iter([b3]),
            &blk_manager,
        );

        check_update_finalized(
            g,
            a2,
            &[a2, a1],
            &[b2, c1, b3],
            HashSet::from_iter([a3]),
            &blk_manager,
        );

        check_update_finalized(
            g,
            a1,
            &[a1],
            &[c1],
            HashSet::from_iter([a3, b3]),
            &blk_manager,
        );

        check_update_finalized(
            a1,
            a2,
            &[a2],
            &[b2, b3],
            HashSet::from_iter([a3]),
            &blk_manager,
        );

        check_update_finalized(
            a1,
            a3,
            &[a3, a2],
            &[b2, b3],
            HashSet::from_iter([a3]),
            &blk_manager,
        );

        check_update_finalized(a3, a3, &[], &[], HashSet::from_iter([a3]), &blk_manager);
    }
}
