//! Types relating to EE block related structures.

use strata_acct_types::{AcctId, Hash, SentMessage, SubjectId};

/// Container for an execution block that signals additional data with it.
// TODO better name, using an intentionally bad one for now
// TODO SSZ
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecBlockNotpackage {
    /// Execution blkid, which commits to things more cleanly.
    exec_blkid: Hash,

    /// Encoded block hash to more cleanly link things.
    block_raw_hash: Hash,

    /// Inputs processed in the block.
    inputs: BlockInputs,

    /// Outputs produced in the block.
    outputs: BlockOutputs,
}

impl ExecBlockNotpackage {
    pub fn exec_blkid(&self) -> [u8; 32] {
        self.exec_blkid
    }

    pub fn block_raw_hash(&self) -> [u8; 32] {
        self.block_raw_hash
    }

    pub fn inputs(&self) -> &BlockInputs {
        &self.inputs
    }

    pub fn outputs(&self) -> &BlockOutputs {
        &self.outputs
    }
}

/// Inputs from the OL to the EE processed in a single EE block.
// TODO SSZ
// TODO builder
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockInputs {
    subject_deposits: Vec<SubjectDepositData>,
}

/// Describes data for a simple deposit to a subject within an EE.
///
/// This is used for deposits from L1, but can encompass any "blind" transfer to
/// a subject (which doesn't allow it to autonomously respond to the deposit or
/// know where the sender was).
// TODO SSZ
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubjectDepositData {
    dest: SubjectId,
    value: u64,
}

impl SubjectDepositData {
    pub fn new(dest: SubjectId, value: u64) -> Self {
        Self { dest, value }
    }

    pub fn dest(&self) -> SubjectId {
        self.dest
    }

    pub fn value(&self) -> u64 {
        self.value
    }
}

/// Outputs from an EE to the OL produced in a single EE block.
// TODO SSZ
// TODO builder type
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockOutputs {
    output_transfers: Vec<OutputTransfer>,
    output_messages: Vec<SentMessage>,
}

// TODO SSZ?
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputTransfer {
    dest: AcctId,
    value: u64,
}
