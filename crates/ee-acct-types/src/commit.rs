//! Commit operation types.

use strata_acct_types::Hash;
use strata_ee_chain_types::ExecBlockNotpackage;

use crate::errors::EnvError;

/// Chain segment data provided with a coinput.
#[derive(Clone, Debug)]
pub struct CommitChainSegment {
    blocks: Vec<CommitBlockData>,
}

impl CommitChainSegment {
    pub fn new(blocks: Vec<CommitBlockData>) -> Self {
        Self { blocks }
    }

    pub fn decode(_buf: &[u8]) -> Result<Self, EnvError> {
        // TODO
        unimplemented!()
    }

    pub fn blocks(&self) -> &[CommitBlockData] {
        &self.blocks
    }

    /// Gets the new exec tip blkid that we would refer to the chain segment
    /// by in a commit.
    pub fn new_exec_tip_blkid(&self) -> Option<Hash> {
        self.blocks.last().map(|b| b.notpackage().exec_blkid())
    }
}

/// Data for a particular EE block linking it in with the chain.
#[derive(Clone, Debug)]
pub struct CommitBlockData {
    notpackage: ExecBlockNotpackage,
    raw_full_block: Vec<u8>,
}

impl CommitBlockData {
    pub fn new(notpackage: ExecBlockNotpackage, raw_full_block: Vec<u8>) -> Self {
        Self {
            notpackage,
            raw_full_block,
        }
    }

    pub fn notpackage(&self) -> &ExecBlockNotpackage {
        &self.notpackage
    }

    pub fn raw_full_block(&self) -> &[u8] {
        &self.raw_full_block
    }
}
