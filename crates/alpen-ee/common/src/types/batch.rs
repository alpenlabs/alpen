use std::iter;

use bitcoin::{Txid, Wtxid};
use strata_acct_types::Hash;
use strata_identifiers::L1BlockCommitment;

use crate::ProofId;

/// Unique, deterministic identifier for an Batch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BatchId {
    prev_block: Hash,
    last_block: Hash,
}

impl BatchId {
    fn new(prev_block: Hash, last_block: Hash) -> Self {
        Self {
            prev_block,
            last_block,
        }
    }
}

/// Batch-DA related data in an L1 block
#[derive(Debug, Clone)]
pub struct L1DaBlockRef {
    /// L1 block holding DA txns.
    pub block: L1BlockCommitment,
    /// relevant transactions in this block.
    pub txns: Vec<(Txid, Wtxid)>,
    // inclusion merkle proof ?
}

/// Batch lifecycle states
#[derive(Debug, Clone)]
pub enum BatchStatus {
    /// Newly created
    Init,
    /// DA txn(s) posted, waiting for inclusion in block.
    DaPending { txns: Vec<(Txid, Wtxid)> },
    /// DA txn(s) included in block(s).
    DaComplete { da: Vec<L1DaBlockRef> },
    /// Proving started, waiting for proof generation.
    ProofPending {
        da: Vec<L1DaBlockRef>,
        proof_job_id: String,
    },
    /// Proof ready. Update ready to be posted to OL.
    ProofReady {
        da: Vec<L1DaBlockRef>,
        proof: ProofId,
    },
}

/// Represents a sequence of blocks that are treated as a unit for DA and posting updates to OL.
#[derive(Debug)]
pub struct Batch {
    /// Sequential update index, also used in
    idx: u64,
    /// last block of (idx - 1)th update
    prev_block: Hash,
    /// last block in this update
    last_block: Hash,
    /// rest of the blocks in this update.
    /// cached here for easier processing.
    inner_blocks: Vec<Hash>,
}

impl Batch {
    /// Create a new Batch.
    pub fn new(idx: u64, prev_block: Hash, last_block: Hash, inner_blocks: Vec<Hash>) -> Self {
        debug_assert_ne!(prev_block, last_block);
        Self {
            idx,
            prev_block,
            last_block,
            inner_blocks,
        }
    }

    /// Get deterministic id.
    pub fn id(&self) -> BatchId {
        BatchId::new(self.prev_block, self.last_block)
    }

    /// Get sequential index.
    /// This should equal the sequence number in account update sent to OL.
    pub fn idx(&self) -> u64 {
        self.idx
    }

    /// last block of the previous Batch.
    pub fn prev_block(&self) -> Hash {
        self.prev_block
    }

    /// last block of this Batch
    pub fn last_block(&self) -> Hash {
        self.last_block
    }

    /// Iterate over all blocks in range of this Batch.
    pub fn blocks_iter(&self) -> impl Iterator<Item = Hash> + '_ {
        self.inner_blocks
            .iter()
            .copied()
            .chain(iter::once(self.last_block()))
    }
}
