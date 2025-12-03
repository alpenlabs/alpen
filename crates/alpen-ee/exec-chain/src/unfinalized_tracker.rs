use std::collections::HashMap;

use alpen_ee_common::ExecBlockRecord;
use eyre::eyre;
use strata_acct_types::Hash;

#[derive(Debug, Clone, Copy)]
pub(crate) struct BlockNumHash {
    pub hash: Hash,
    pub height: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct BlockEntry {
    pub blocknum: u64,
    pub blockhash: Hash,
    pub parent: Hash,
}

impl From<&ExecBlockRecord> for BlockEntry {
    fn from(value: &ExecBlockRecord) -> Self {
        Self {
            blockhash: value.blockhash(),
            blocknum: value.blocknum(),
            parent: value.parent_blockhash(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct UnfinalizedTracker {
    finalized: BlockNumHash,
    best: BlockNumHash,
    tips: HashMap<Hash, u64>,
    blocks: HashMap<Hash, BlockEntry>,
}

/// Possible results of attaching block to [`UnfinalizedTracker`].
pub(crate) enum AttachBlockRes {
    /// Attached successfully.
    Ok(Hash),
    /// Block already exists.
    ExistingBlock,
    /// Block is below finalized height, cannot be attached.
    BelowFinalized(BlockEntry),
    /// Block does not extend any existing tip, cannot be attached.
    OrphanBlock(BlockEntry),
}

impl UnfinalizedTracker {
    pub(crate) fn new_empty(finalized_block: BlockEntry) -> Self {
        let hash = finalized_block.blockhash;
        let height = finalized_block.blocknum;
        Self {
            finalized: BlockNumHash { hash, height },
            best: BlockNumHash { hash, height },
            tips: HashMap::from([(hash, height)]),
            blocks: HashMap::from([(hash, finalized_block)]),
        }
    }

    pub(crate) fn attach_block(&mut self, block: BlockEntry) -> AttachBlockRes {
        // 1. Is it an existing block ?
        let block_hash = block.blockhash;
        if self.blocks.contains_key(&block_hash) {
            return AttachBlockRes::ExistingBlock;
        }

        // 2. Is it below finalized ?
        let block_height = block.blocknum;
        if block_height < self.finalized.height {
            return AttachBlockRes::BelowFinalized(block);
        }

        // 3. Does it extend an existing tip ?
        let parent_blockhash = block.parent;
        if self.tips.contains_key(&parent_blockhash) {
            self.blocks.insert(block_hash, block);
            self.tips.remove(&parent_blockhash);
            self.tips.insert(block_hash, block_height);

            (self.best.hash, self.best.height) = self.compute_best_tip();
            return AttachBlockRes::Ok(self.best.hash);
        };

        // 4. does it create a new tip ?
        if self.blocks.contains_key(&parent_blockhash) {
            self.blocks.insert(block_hash, block);
            self.tips.insert(block_hash, block_height);

            (self.best.hash, self.best.height) = self.compute_best_tip();
            return AttachBlockRes::Ok(self.best.hash);
        }

        // does not extend any known block
        AttachBlockRes::OrphanBlock(block)
    }

    fn compute_best_tip(&self) -> (Hash, u64) {
        let height = self.tips.get(&self.best.hash).expect("entry must exist");
        let (hash, height) = self.tips.iter().fold(
            (&self.best.hash, height),
            |(a_hash, a_height), (b_hash, b_height)| {
                if b_height > a_height {
                    (b_hash, b_height)
                } else {
                    (a_hash, a_height)
                }
            },
        );
        (*hash, *height)
    }

    pub(crate) fn contains_block(&self, hash: &Hash) -> bool {
        self.blocks.contains_key(hash)
    }

    pub(crate) fn finalized(&self) -> BlockNumHash {
        self.finalized
    }

    pub(crate) fn best(&self) -> BlockNumHash {
        self.best
    }

    pub(crate) fn prune_finalized(&mut self, new_finalized: Hash) -> eyre::Result<FinalizeReport> {
        if new_finalized == self.finalized.hash {
            // noop
            return Ok(FinalizeReport::new_empty());
        }

        let Some(new_finalized_block) = self.blocks.remove(&new_finalized) else {
            // unknown block
            return Err(eyre!("unknown block: {:?}", new_finalized));
        };

        // get all blocks that are newly finalized
        let finalized_blocks_count = new_finalized_block.blocknum - self.finalized.height;
        let mut finalized_hashes = Vec::<Hash>::with_capacity(finalized_blocks_count as usize);
        let mut block = new_finalized_block.clone();
        for _ in 0..finalized_blocks_count {
            finalized_hashes.push(block.blockhash);
            block = self.blocks.remove(&block.parent).expect("should exist");
        }

        // sanity check
        if block.blockhash != self.finalized.hash {
            return Err(eyre!("invalid tracker state"));
        }

        // easier to just recreate the tracker using existing blocks
        let mut tmp_tracker = Self::new_empty(new_finalized_block);
        let mut blocks = self.blocks.drain().collect::<Vec<_>>();

        blocks.sort_by_cached_key(|(_, block)| block.blocknum);
        let mut removed = Vec::new();

        for (_, block) in blocks {
            match tmp_tracker.attach_block(block) {
                AttachBlockRes::OrphanBlock(block) => {
                    removed.push(block.blockhash);
                }
                AttachBlockRes::BelowFinalized(block) => {
                    removed.push(block.blockhash);
                }
                AttachBlockRes::Ok(_) => {}
                _ => unreachable!(),
            }
        }

        *self = tmp_tracker;

        finalized_hashes.reverse();
        Ok(FinalizeReport::new(finalized_hashes, removed))
    }
}

#[derive(Debug)]
pub(crate) struct FinalizeReport {
    /// blocks that are now in finalized chain
    pub(crate) finalize: Vec<Hash>,
    /// blocks that need not be tracked any longer
    pub(crate) remove: Vec<Hash>,
}

impl FinalizeReport {
    fn new(finalize: Vec<Hash>, remove: Vec<Hash>) -> Self {
        Self { finalize, remove }
    }

    fn new_empty() -> Self {
        Self {
            finalize: Vec::new(),
            remove: Vec::new(),
        }
    }
}
