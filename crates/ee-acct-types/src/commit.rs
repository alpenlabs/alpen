//! Commit operation types.

use strata_ee_chain_types::ExecBlockNotpackage;

#[derive(Clone, Debug)]
pub struct CommitCoinput {
    blocks: Vec<CommitBlockData>,
    raw_prev_header: Vec<u8>,
    raw_partial_state: Vec<u8>,
}

impl CommitCoinput {
    pub fn new(
        blocks: Vec<CommitBlockData>,
        raw_prev_header: Vec<u8>,
        raw_partial_state: Vec<u8>,
    ) -> Self {
        Self {
            blocks,
            raw_prev_header,
            raw_partial_state,
        }
    }

    pub fn decode_raw(buf: &[u8]) -> Option<CommitCoinput> {
        // TODO implement this function properly
        unimplemented!()
    }

    pub fn blocks(&self) -> &[CommitBlockData] {
        &self.blocks
    }

    pub fn raw_prev_header(&self) -> &[u8] {
        &self.raw_prev_header
    }

    pub fn raw_partial_state(&self) -> &[u8] {
        &self.raw_partial_state
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
