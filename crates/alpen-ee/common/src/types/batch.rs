use std::iter;

use bitcoin::{Txid, Wtxid};
use strata_acct_types::Hash;
use strata_identifiers::L1BlockCommitment;

use crate::{BlockNumHash, ProofId};

/// Unique, deterministic identifier for a batch.
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

    /// Create a BatchId from its component parts.
    pub fn from_parts(prev_block: Hash, last_block: Hash) -> Self {
        Self::new(prev_block, last_block)
    }

    /// Get the prev_block component.
    pub fn prev_block(&self) -> Hash {
        self.prev_block
    }

    /// Get the last_block component.
    pub fn last_block(&self) -> Hash {
        self.last_block
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
    /// Newly sealed batch.
    Sealed,
    /// DA txn(s) posted, waiting for inclusion in block.
    DaPending { txns: Vec<(Txid, Wtxid)> },
    /// DA txn(s) included in block(s).
    DaComplete { da: Vec<L1DaBlockRef> },
    /// Proving started, waiting for proof generation.
    ProofPending { da: Vec<L1DaBlockRef> },
    /// Proof ready. Update ready to be posted to OL.
    ProofReady {
        da: Vec<L1DaBlockRef>,
        proof: ProofId,
    },
}

/// Represents a sequence of blocks that are treated as a unit for DA and posting updates to OL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    /// Sequential batch index.
    idx: u64,
    /// Last block of (idx - 1)th batch.
    prev_block: Hash,
    /// Last block in this batch.
    last_block: Hash,
    /// Blocknum of last block in this batch
    last_blocknum: u64,
    /// Rest of the blocks in this batch, cached here for easier processing.
    inner_blocks: Vec<Hash>,
}

impl Batch {
    /// Create a new batch.
    pub fn new(
        idx: u64,
        prev_block: Hash,
        last_block: Hash,
        last_blocknum: u64,
        inner_blocks: Vec<Hash>,
    ) -> Result<Self, &'static str> {
        if idx == 0 {
            return Err("non-genesis batch cannot have idx == 0");
        }
        if prev_block.is_zero() {
            return Err("non-genesis batch cannot have ZERO prev_block");
        }
        if last_block.is_zero() {
            return Err("batch cannot have ZERO last_block");
        }
        if prev_block == last_block {
            return Err("batch cannot be empty");
        }
        Ok(Self {
            idx,
            prev_block,
            last_block,
            last_blocknum,
            inner_blocks,
        })
    }

    /// Create genesis batch.
    ///
    /// Genesis batch is a special marker, which must always exist in storage, defined as a batch
    /// with idx == 0 AND prev_block == ZERO and last_block == genesis block. A genesis batch must
    /// always exist in storage. This is mainly to make reorg related operations simpler.
    pub fn new_genesis_batch(
        genesis_hash: Hash,
        genesis_blocknum: u64,
    ) -> Result<Self, &'static str> {
        if genesis_hash.is_zero() {
            return Err("genesis block cannot be ZERO");
        }

        Ok(Self {
            idx: 0,
            prev_block: Hash::zero(),
            last_block: genesis_hash,
            last_blocknum: genesis_blocknum,
            inner_blocks: Vec::new(),
        })
    }

    pub fn is_genesis_batch(&self) -> bool {
        self.idx() == 0
    }

    /// Get deterministic id.
    pub fn id(&self) -> BatchId {
        BatchId::new(self.prev_block, self.last_block)
    }

    /// Get sequential index.
    pub fn idx(&self) -> u64 {
        self.idx
    }

    /// last block of the previous batch.
    pub fn prev_block(&self) -> Hash {
        self.prev_block
    }

    /// last block of this batch.
    pub fn last_block(&self) -> Hash {
        self.last_block
    }

    pub fn last_blocknum(&self) -> u64 {
        self.last_blocknum
    }

    pub fn last_blocknumhash(&self) -> BlockNumHash {
        BlockNumHash::new(self.last_block(), self.last_blocknum())
    }

    /// Get the inner blocks (blocks between prev_block and last_block, exclusive of last_block).
    pub fn inner_blocks(&self) -> &[Hash] {
        &self.inner_blocks
    }

    /// Iterate over all blocks in range of this batch.
    pub fn blocks_iter(&self) -> impl Iterator<Item = Hash> + '_ {
        self.inner_blocks
            .iter()
            .copied()
            .chain(iter::once(self.last_block()))
    }
}
