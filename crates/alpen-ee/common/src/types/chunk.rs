use std::iter;

use strata_acct_types::Hash;

use crate::ProofId;

/// Lifecycle states for chunk
#[derive(Debug, Clone)]
pub enum ChunkStatus {
    /// Proving has not started yet.
    ProvingNotStarted,
    /// Proving started. Pending proof generation.
    ProofPending(String),
    /// Valid proof ready.
    ProofReady(ProofId),
}

/// Unique, deterministic identifier for a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkId {
    prev_block: Hash,
    last_block: Hash,
}

impl ChunkId {
    fn new(prev_block: Hash, last_block: Hash) -> Self {
        Self {
            prev_block,
            last_block,
        }
    }
}

/// Represents a sequence of blocks that are processed together as a unit during proving.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Sequential chunk index.
    idx: u64,
    /// Last block of (idx - 1)th chunk that this chunk extends.
    prev_block: Hash,
    /// Last block of this chunk. A chunk cannot be empty.
    last_block: Hash,
    /// Rest of the blocks in the chunk.
    inner_blocks: Vec<Hash>,
}

impl Chunk {
    /// Create a new chunk.
    pub fn new(idx: u64, prev_block: Hash, last_block: Hash, inner_blocks: Vec<Hash>) -> Self {
        debug_assert_ne!(prev_block, last_block);
        Self {
            idx,
            prev_block,
            last_block,
            inner_blocks,
        }
    }

    /// Deterministic chunk id.
    pub fn id(&self) -> ChunkId {
        ChunkId::new(self.prev_block, self.last_block)
    }

    /// Sequential chunk index.
    pub fn idx(&self) -> u64 {
        self.idx
    }

    /// Last block of (idx - 1)th chunk, that this chunk extends.
    pub fn prev_block(&self) -> Hash {
        self.prev_block
    }

    /// Last block of this chunk.
    pub fn last_block(&self) -> Hash {
        self.last_block
    }

    /// Iterate over all blocks in this chunk.
    pub fn blocks_iter(&self) -> impl Iterator<Item = Hash> + '_ {
        self.inner_blocks
            .iter()
            .copied()
            .chain(iter::once(self.last_block()))
    }
}
