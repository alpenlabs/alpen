use alpen_ee_primitives::{indexed_vec::IndexedVec, EEBlockHash};
use reth_primitives::SealedBlock;

use crate::message::{InboundMsgEnvelope, OutboundMsgEnvelope};

/// Maps 1-1 to an eth block and holds additional block production metadata
#[derive(Debug, Clone)]
pub struct BlockMetadata {
    /// blocknumber
    number: u64,
    /// Blockhash of ee block
    blockhash: EEBlockHash,
    /// Blockhash of parent ee block
    parent_blockhash: EEBlockHash,
    /// All input messages must be in correct order without any gaps
    input_msgs: IndexedVec<InboundMsgEnvelope>,
    output_msg: Vec<OutboundMsgEnvelope>,
}

impl BlockMetadata {
    pub fn new(
        number: u64,
        blockhash: EEBlockHash,
        parent_blockhash: EEBlockHash,
        input_msgs: IndexedVec<InboundMsgEnvelope>,
        output_msg: Vec<OutboundMsgEnvelope>,
    ) -> Self {
        Self {
            number,
            blockhash,
            parent_blockhash,
            input_msgs,
            output_msg,
        }
    }

    pub fn number(&self) -> u64 {
        self.number
    }

    pub fn blockhash(&self) -> &EEBlockHash {
        &self.blockhash
    }

    pub fn parent_blockhash(&self) -> &EEBlockHash {
        &self.parent_blockhash
    }

    pub fn input_msgs(&self) -> &IndexedVec<InboundMsgEnvelope> {
        &self.input_msgs
    }

    pub fn output_msg(&self) -> &Vec<OutboundMsgEnvelope> {
        &self.output_msg
    }
}

/// Wrapper over built ee block
#[derive(Debug, Clone)]
pub struct BlockPayload(SealedBlock);

impl From<SealedBlock> for BlockPayload {
    fn from(value: SealedBlock) -> Self {
        Self(value)
    }
}

impl BlockPayload {
    pub fn block(&self) -> &SealedBlock {
        &self.0
    }
}
