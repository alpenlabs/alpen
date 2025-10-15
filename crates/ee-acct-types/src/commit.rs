//! Commit operation types.

use strata_acct_types::Hash;
use strata_ee_chain_types::ExecBlockPackage;

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
        self.blocks.last().map(|b| b.package().exec_blkid())
    }
}

/// Data for a particular EE block linking it in with the chain.
#[derive(Clone, Debug)]
pub struct CommitBlockData {
    package: ExecBlockPackage,
    raw_full_block: Vec<u8>,
}

impl CommitBlockData {
    pub fn new(package: ExecBlockPackage, raw_full_block: Vec<u8>) -> Self {
        Self {
            package,
            raw_full_block,
        }
    }

    pub fn package(&self) -> &ExecBlockPackage {
        &self.package
    }

    pub fn raw_full_block(&self) -> &[u8] {
        &self.raw_full_block
    }
}
