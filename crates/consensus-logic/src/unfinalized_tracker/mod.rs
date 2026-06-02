//! Tracker for keeping track of the tree of unfinalized blocks.

use std::collections::*;

use strata_identifiers::Slot;
use strata_primitives::{buf::Buf32, epoch::EpochCommitment, l2::L2BlockCommitment};
use strata_state::prelude::*;
use tracing::*;

use crate::errors::ChainTipError;

mod ol;

pub use ol::UnfinalizedOLBlockSource;

/// Entry in block tracker table we use to relate a block with its immediate
/// relatives.
#[derive(Debug)]
struct BlockEntry {
    slot: u64,
    parent: L2BlockId,
    children: HashSet<L2BlockId>,
}

/// Tracks the unfinalized block tree on top of the finalized tip.
#[derive(Debug)]
pub struct UnfinalizedBlockTracker {
    /// Block that we treat as a base that all of the other blocks that we're
    /// considering uses.
    finalized_epoch: EpochCommitment,

    /// Table of pending blocks near the tip of the block tree.
    pending_table: HashMap<L2BlockId, BlockEntry>,

    /// Unfinalized chain tips.  This also includes the finalized tip if there's
    /// no pending blocks.
    unfinalized_tips: HashSet<L2BlockId>,
}

impl UnfinalizedBlockTracker {
    /// Creates a new tracker with just a finalized tip and no pending blocks.
    pub fn new_empty(finalized_epoch: EpochCommitment) -> Self {
        let fin_tip = *finalized_epoch.last_blkid();

        let mut pending_tbl = HashMap::new();
        pending_tbl.insert(
            fin_tip,
            BlockEntry {
                slot: finalized_epoch.last_slot(),
                parent: L2BlockId::from(Buf32::zero()),
                children: HashSet::new(),
            },
        );

        let mut unf_tips = HashSet::new();
        unf_tips.insert(fin_tip);
        Self {
            finalized_epoch,
            pending_table: pending_tbl,
            unfinalized_tips: unf_tips,
        }
    }

    /// Returns the finalized epoch that we build blocks off of.
    pub fn finalized_epoch(&self) -> &EpochCommitment {
        &self.finalized_epoch
    }

    /// Returns the "finalized tip", which is the terminal block of the
    /// finalized epoch and the base of the unfinalized tree.
    pub fn finalized_tip(&self) -> &L2BlockId {
        self.finalized_epoch.last_blkid()
    }

    /// Returns `true` if the block is either the finalized tip or is already
    /// known to the tracker.
    pub fn is_seen_block(&self, id: &L2BlockId) -> bool {
        self.finalized_tip() == id || self.pending_table.contains_key(id)
    }

    /// Returns the slot of some block, if present in the tracker.
    pub fn get_slot(&self, id: &L2BlockId) -> Option<u64> {
        self.pending_table.get(id).map(|ent| ent.slot)
    }

    /// Gets the parent of a block from within the tree.  Returns `None` if the
    /// block or its parent isn't in the tree.  Returns `None` for the finalized
    /// tip block, since its parent isn't in the tree.
    pub fn get_parent(&self, id: &L2BlockId) -> Option<&L2BlockId> {
        if id == self.finalized_tip() {
            return None;
        }
        self.pending_table.get(id).map(|ent| &ent.parent)
    }

    /// Returns an iterator over the chain tips.
    pub fn chain_tips_iter(&self) -> impl Iterator<Item = &L2BlockId> {
        self.unfinalized_tips.iter()
    }

    /// Returns an iterator over the block commitments of the chain tips.
    pub fn chain_tip_blocks_iter(&self) -> impl Iterator<Item = L2BlockCommitment> + '_ {
        self.chain_tips_iter()
            .map(|id| L2BlockCommitment::new(self.pending_table[id].slot, *id))
    }

    /// Checks if the block is traceable all the way back to the finalized tip.
    fn sanity_check_parent_seq(&self, blkid: &L2BlockId) -> bool {
        if blkid == self.finalized_tip() {
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
        slot: Slot,
        blkid: L2BlockId,
        parent_blkid: L2BlockId,
    ) -> Result<bool, ChainTipError> {
        if self.pending_table.contains_key(&blkid) {
            warn!(?blkid, "block already attached");
            return Ok(false);
        }

        if let Some(parent_ent) = self.pending_table.get_mut(&parent_blkid) {
            if slot <= parent_ent.slot {
                return Err(ChainTipError::ChildBeforeParent(slot, parent_ent.slot));
            }

            parent_ent.children.insert(blkid);
        } else {
            return Err(ChainTipError::AttachMissingParent(blkid, parent_blkid));
        }

        let ent = BlockEntry {
            slot,
            parent: parent_blkid,
            children: HashSet::new(),
        };

        self.pending_table.insert(blkid, ent);

        // Also update the tips table, removing the parent if it's there.
        let did_replace = self.unfinalized_tips.remove(&parent_blkid);
        self.unfinalized_tips.insert(blkid);

        Ok(!did_replace)
    }

    /// Updates the finalized block tip, returning a report that includes the
    /// precise blocks that were finalized transatively and any blocks on
    /// competing chains that were rejected.
    pub fn update_finalized_epoch(
        &mut self,
        epoch: &EpochCommitment,
    ) -> Result<FinalizeReport, ChainTipError> {
        let blkid = epoch.last_blkid();

        // Sanity check the block so we know it's here.
        if !self.sanity_check_parent_seq(blkid) {
            return Err(ChainTipError::MissingBlock(*blkid));
        }

        if blkid == self.finalized_tip() {
            return Ok(FinalizeReport {
                old_epoch: self.finalized_epoch,
                finalized: vec![*blkid],
                rejected: Vec::new(),
            });
        }

        let mut finalized = vec![];
        let mut at = *blkid;

        // Walk down to the current finalized tip and put everything as finalized.
        while at != *self.finalized_tip() {
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
            if at == *self.finalized_tip() {
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
        let old_epoch = self.finalized_epoch;
        self.finalized_epoch = *epoch;

        // Sanity check and construct the report.
        assert!(
            !finalized.is_empty(),
            "chaintip: somehow finalized no blocks"
        );
        Ok(FinalizeReport {
            old_epoch,
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

    #[cfg(test)]
    pub fn unchecked_set_finalized_tip(&mut self, epoch: EpochCommitment) {
        self.finalized_epoch = epoch;
    }

    #[cfg(test)]
    pub fn insert_fake_block(&mut self, slot: u64, id: L2BlockId, parent: L2BlockId) {
        let ent = BlockEntry {
            slot,
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
    /// Previously finalized epoch.
    old_epoch: EpochCommitment,

    /// Block we've newly finalized.  The first one of this
    finalized: Vec<L2BlockId>,

    /// Any blocks that were on competing chains than the one we finalized.
    rejected: Vec<L2BlockId>,
}

impl FinalizeReport {
    /// Returns the blkid that was the previously finalized tip.  It's still
    /// finalized, but there's newer blocks that are also finalized now.
    pub fn prev_tip(&self) -> &L2BlockId {
        self.old_epoch.last_blkid()
    }

    /// The new chain tip that's finalized now.
    pub fn new_tip(&self) -> &L2BlockId {
        if self.finalized.is_empty() {
            self.prev_tip()
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
