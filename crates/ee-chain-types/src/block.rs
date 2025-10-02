//! Types relating to EE block related structures.

use strata_acct_types::{AccountId, BitcoinAmount, Hash, SentMessage, SubjectId};

/// Container for an execution block that signals additional data with it.
// TODO better name, using an intentionally bad one for now
// TODO SSZ
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecBlockNotpackage {
    /// Commitment to the block itself.
    commitment: ExecBlockCommitment,

    /// Inputs processed in the block.
    inputs: BlockInputs,

    /// Outputs produced in the block.
    outputs: BlockOutputs,
}

impl ExecBlockNotpackage {
    pub fn new(
        commitment: ExecBlockCommitment,
        inputs: BlockInputs,
        outputs: BlockOutputs,
    ) -> Self {
        Self {
            commitment,
            inputs,
            outputs,
        }
    }

    pub fn commitment(&self) -> &ExecBlockCommitment {
        &self.commitment
    }

    pub fn exec_blkid(&self) -> [u8; 32] {
        self.commitment().exec_blkid()
    }

    pub fn raw_block_encoded_hash(&self) -> [u8; 32] {
        self.commitment().raw_block_encoded_hash()
    }

    pub fn inputs(&self) -> &BlockInputs {
        &self.inputs
    }

    pub fn outputs(&self) -> &BlockOutputs {
        &self.outputs
    }
}

/// Commitment to a particular execution block, in multiple ways.
// should this contain parent and index information?
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ExecBlockCommitment {
    /// Block ID as interpreted by the execution environment, probably a hash of
    /// a block header.
    ///
    /// This is so that the proofs are able to cheaply reason about the chain,
    /// using its native concepts.
    ///
    /// We can't *just* use `raw_block_encoded_hash`, because we would have to
    /// include the full block in the proof, and that doesn't even give us
    /// parent linkages.
    exec_blkid: Hash,

    /// Hash of the encoded block.
    ///
    /// This is so that we can know if we have the right block without knowing
    /// how to parse it.
    ///
    /// We can't *just* use `exec_blkid`, because we might not be in a context
    /// where we know how to parse a block in order to hash it.
    raw_block_encoded_hash: Hash,
}

impl ExecBlockCommitment {
    pub fn new(exec_blkid: Hash, raw_block_encoded_hash: Hash) -> Self {
        Self {
            exec_blkid,
            raw_block_encoded_hash,
        }
    }

    pub fn exec_blkid(&self) -> [u8; 32] {
        self.exec_blkid
    }

    pub fn raw_block_encoded_hash(&self) -> [u8; 32] {
        self.raw_block_encoded_hash
    }
}

/// Inputs from the OL to the EE processed in a single EE block.
// TODO SSZ
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockInputs {
    subject_deposits: Vec<SubjectDepositData>,
}

impl BlockInputs {
    fn new(subject_deposits: Vec<SubjectDepositData>) -> Self {
        Self { subject_deposits }
    }

    /// Creates a new empty instance.
    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn subject_deposits(&self) -> &[SubjectDepositData] {
        &self.subject_deposits
    }

    pub fn add_subject_deposit(&mut self, d: SubjectDepositData) {
        self.subject_deposits.push(d);
    }

    /// Returns the total number of inputs across all types.
    pub fn total_inputs(&self) -> usize {
        self.subject_deposits.len()
    }
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
    value: BitcoinAmount,
}

impl SubjectDepositData {
    pub fn new(dest: SubjectId, value: BitcoinAmount) -> Self {
        Self { dest, value }
    }

    pub fn dest(&self) -> SubjectId {
        self.dest
    }

    pub fn value(&self) -> BitcoinAmount {
        self.value
    }
}

/// Outputs from an EE to the OL produced in a single EE block.
// TODO SSZ
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockOutputs {
    output_transfers: Vec<OutputTransfer>,
    output_messages: Vec<SentMessage>,
}

impl BlockOutputs {
    fn new(output_transfers: Vec<OutputTransfer>, output_messages: Vec<SentMessage>) -> Self {
        Self {
            output_transfers,
            output_messages,
        }
    }

    /// Creates a new empty instance.
    pub fn new_empty() -> Self {
        Self::new(Vec::new(), Vec::new())
    }

    pub fn output_transfers(&self) -> &[OutputTransfer] {
        &self.output_transfers
    }

    /// Adds a transfer output.
    pub fn add_transfer(&mut self, t: OutputTransfer) {
        self.output_transfers.push(t);
    }

    pub fn output_messages(&self) -> &[SentMessage] {
        &self.output_messages
    }

    /// Adds a message output.
    pub fn add_message(&mut self, m: SentMessage) {
        self.output_messages.push(m);
    }
}

// TODO SSZ?
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputTransfer {
    /// Destination orchestration layer account ID.
    dest: AccountId,

    /// Native asset value sent (satoshis).
    value: BitcoinAmount,
}

impl OutputTransfer {
    pub fn new(dest: AccountId, value: BitcoinAmount) -> Self {
        Self { dest, value }
    }

    pub fn dest(&self) -> AccountId {
        self.dest
    }

    pub fn value(&self) -> BitcoinAmount {
        self.value
    }
}
