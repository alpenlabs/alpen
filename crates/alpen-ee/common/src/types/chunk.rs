use std::iter;

use strata_acct_types::Hash;

/// Lifecycle states for chunk
#[derive(Debug)]
pub enum ChunkStatus {
    /// Proving has not started yet.
    Unproven,
    /// Proving started. Pending proof generation.
    PendingProof(String),
    /// Valid proof ready.
    /// TODO: correct proof type
    ProofReady(Vec<u8>),
}

/// Unique identifier for a chunk
#[derive(Debug)]
pub struct ChunkId {
    pub prev_block: Hash,
    pub last_block: Hash,
}

/// Represents a sequence of blocks that are processed together as a unit during proving.
#[derive(Debug)]
pub struct Chunk {
    /// Sequential chunk index
    idx: u64,
    /// last block of (idx - 1)th chunk that this chunk extends
    prev_block: Hash,
    /// last block of this chunk. A chunk cannot be empty.
    last_block: Hash,
    /// rest of the blocks in the chunk.
    blocks: Vec<Hash>,
    /// status
    proof_status: ChunkStatus,
}

impl Chunk {
    pub fn new_unproven(idx: u64, prev_block: Hash, last_block: Hash, blocks: Vec<Hash>) -> Self {
        Self {
            idx,
            prev_block,
            last_block,
            blocks,
            proof_status: ChunkStatus::Unproven,
        }
    }

    pub fn set_proof_pending(&mut self, proof_task_id: String) {
        self.proof_status = ChunkStatus::PendingProof(proof_task_id);
    }

    pub fn set_proof(&mut self, proof: Vec<u8>) {
        self.proof_status = ChunkStatus::ProofReady(proof);
    }

    pub fn id(&self) -> ChunkId {
        ChunkId {
            prev_block: self.prev_block,
            last_block: self.last_block,
        }
    }

    pub fn idx(&self) -> u64 {
        self.idx
    }

    pub fn prev_block(&self) -> Hash {
        self.prev_block
    }

    pub fn last_block(&self) -> Hash {
        self.last_block
    }

    pub fn blocks_iter(&self) -> impl Iterator<Item = Hash> + '_ {
        self.blocks
            .iter()
            .copied()
            .chain(iter::once(self.last_block()))
    }
}
