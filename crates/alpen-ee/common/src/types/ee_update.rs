use std::iter;

use bitcoin::Txid;
use strata_acct_types::Hash;
use strata_identifiers::L1BlockCommitment;

use crate::ProofId;

/// Unique identifier for an EeUpdate
#[derive(Debug)]
pub struct EeUpdateId {
    pub prev_block: Hash,
    pub last_block: Hash,
}

/// DA related data relevant to EeUpdate
#[derive(Debug)]
pub struct L1DaRef {
    pub block: L1BlockCommitment,
    pub txns: Vec<Txid>,
    // inclusion merkle proof ?
}

/// EeUpdate lifecycle states
#[derive(Debug)]
pub enum EeUpdateStatus {
    /// Newly created
    Init,
    /// DA started, waiting for block
    DaPending { txids: Vec<Txid> },
    /// DA complete
    DaComplete { da: Vec<L1DaRef> },
    /// Proving started, waiting for proof
    ProofPending {
        da: Vec<L1DaRef>,
        proof_job_id: String,
    },
    /// Proof ready
    ProofReady { da: Vec<L1DaRef>, proof: ProofId },
}

/// Represents a sequence of blocks that are treated as a unit for DA and posting updates to OL.
#[derive(Debug)]
pub struct EeUpdate {
    /// Sequential update index, also used in
    idx: u64,
    /// last block of (idx - 1)th update
    prev_block: Hash,
    /// last block in this update
    last_block: Hash,
    /// rest of the blocks in this update.
    /// cached here for easier processing.
    blocks: Vec<Hash>,
    /// Status
    status: EeUpdateStatus,
}

impl EeUpdate {
    pub fn id(&self) -> EeUpdateId {
        EeUpdateId {
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

    pub fn status(&self) -> &EeUpdateStatus {
        &self.status
    }
}
