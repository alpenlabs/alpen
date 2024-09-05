use std::collections::VecDeque;

use bitcoin::BlockHash;

/// State we use in various parts of the reader.
#[derive(Debug)]
pub struct ReaderState {
    /// The highest block in the chain, at `.back()` of queue + 1.
    next_height: u64,

    /// The `.back()` of this should have the same height as cur_height.
    recent_blocks: VecDeque<BlockHash>,

    /// Depth at which we start pulling recent blocks out of the front of the queue.
    max_depth: usize,
}

impl ReaderState {
    /// Constructs a new reader state instance using some context about how we
    /// want to manage it.
    pub fn new(next_height: u64, max_depth: usize, recent_blocks: VecDeque<BlockHash>) -> Self {
        assert!(!recent_blocks.is_empty());
        Self {
            next_height,
            max_depth,
            recent_blocks,
        }
    }

    pub fn next_height(&self) -> u64 {
        self.next_height
    }

    pub fn best_block(&self) -> &BlockHash {
        self.recent_blocks.back().unwrap()
    }

    pub fn best_block_idx(&self) -> u64 {
        self.next_height - 1
    }

    /// Returns the idx of the deepest block in the reader state.
    #[allow(unused)]
    fn deepest_block(&self) -> u64 {
        self.best_block_idx() - self.recent_blocks.len() as u64 + 1
    }

    /// Accepts a new block and possibly purges a buried one.
    pub fn accept_new_block(&mut self, blkhash: BlockHash) -> Option<BlockHash> {
        let ret = if self.recent_blocks.len() > self.max_depth {
            Some(self.recent_blocks.pop_front().unwrap())
        } else {
            None
        };

        self.recent_blocks.push_back(blkhash);
        self.next_height += 1;
        ret
    }

    /// Gets the blockhash of the given height, if we have it.
    #[allow(unused)]
    pub fn get_height_blkid(&self, height: u64) -> Option<&BlockHash> {
        if height >= self.next_height {
            return None;
        }

        if height < self.deepest_block() {
            return None;
        }

        let off = height - self.deepest_block();
        Some(&self.recent_blocks[off as usize])
    }

    fn revert_tip(&mut self) -> Option<BlockHash> {
        if !self.recent_blocks.is_empty() {
            let back = self.recent_blocks.pop_back().unwrap();
            self.next_height -= 1;
            Some(back)
        } else {
            None
        }
    }

    pub fn rollback_to_height(&mut self, new_height: u64) -> Vec<BlockHash> {
        if new_height > self.next_height {
            panic!("reader: new height greater than cur height");
        }

        let rollback_cnt = self.best_block_idx() - new_height;
        if rollback_cnt >= self.recent_blocks.len() as u64 {
            panic!("reader: tried to rollback past deepest block");
        }

        let mut buf = Vec::new();
        for _ in 0..rollback_cnt {
            let blkhash = self.revert_tip().expect("reader: rollback tip");
            buf.push(blkhash);
        }

        // More sanity checks.
        assert!(!self.recent_blocks.is_empty());
        assert_eq!(self.best_block_idx(), new_height);

        buf
    }

    /// Iterates over the blocks back from the tip, giving both the height and
    /// the blockhash to compare against the chain.
    pub fn iter_blocks_back(&self) -> impl Iterator<Item = (u64, &BlockHash)> {
        let best_blk_idx = self.best_block_idx();
        self.recent_blocks
            .iter()
            .rev()
            .enumerate()
            .map(move |(i, b)| (best_blk_idx - i as u64, b))
    }
}
