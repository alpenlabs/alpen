//! Commit operation types.

use strata_ee_chain_types::ExecBlockNotpackage;

/// Chain segment data provided with a coinput.
#[derive(Clone, Debug)]
pub struct CommitChainSegment {
    blocks: Vec<CommitBlockData>,
}

impl CommitChainSegment {
    pub fn new(blocks: Vec<CommitBlockData>) -> Self {
        Self { blocks }
    }

    pub fn decode_raw(buf: &[u8]) -> Option<CommitChainSegment> {
        // TODO implement this function properly
        unimplemented!()
    }

    pub fn blocks(&self) -> &[CommitBlockData] {
        &self.blocks
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
