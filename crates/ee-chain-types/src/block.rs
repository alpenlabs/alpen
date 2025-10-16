//! Types relating to EE block related structures.

use ssz_derive::{Decode, Encode};
use ssz_types::VariableList;
use strata_acct_types::{AccountId, BitcoinAmount, Hash, SentMessage, SubjectId};
use tree_hash_derive::TreeHash as TreeHashDerive;

/// Variable-length list for subject deposits (max 65536 deposits per block)
type SubjectDepositList = VariableList<SubjectDepositData, 65536>;

/// Variable-length list for output transfers (max 65536 transfers per block)
type OutputTransferList = VariableList<OutputTransfer, 65536>;

/// Variable-length list for output messages (max 65536 messages per block)
type OutputMessageList = VariableList<SentMessage, 65536>;

/// Container for an execution block that signals additional data with it.
// TODO better name, using an intentionally bad one for now
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
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
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Encode, Decode, TreeHashDerive)]
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
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct BlockInputs {
    subject_deposits: SubjectDepositList,
}

impl BlockInputs {
    fn new(subject_deposits: Vec<SubjectDepositData>) -> Self {
        Self {
            subject_deposits: SubjectDepositList::from(subject_deposits),
        }
    }

    /// Creates a new empty instance.
    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn subject_deposits(&self) -> &[SubjectDepositData] {
        &self.subject_deposits
    }

    pub fn add_subject_deposit(&mut self, d: SubjectDepositData) {
        self.subject_deposits
            .push(d)
            .expect("subject_deposits list is full");
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
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
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
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
pub struct BlockOutputs {
    output_transfers: OutputTransferList,
    output_messages: OutputMessageList,
}

impl BlockOutputs {
    fn new(output_transfers: Vec<OutputTransfer>, output_messages: Vec<SentMessage>) -> Self {
        Self {
            output_transfers: OutputTransferList::from(output_transfers),
            output_messages: OutputMessageList::from(output_messages),
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
        self.output_transfers
            .push(t)
            .expect("output_transfers list is full");
    }

    pub fn output_messages(&self) -> &[SentMessage] {
        &self.output_messages
    }

    /// Adds a message output.
    pub fn add_message(&mut self, m: SentMessage) {
        self.output_messages
            .push(m)
            .expect("output_messages list is full");
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode, TreeHashDerive)]
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

#[cfg(test)]
mod tests {
    use ssz::{Decode, Encode};
    use tree_hash::TreeHash;

    use super::*;

    #[test]
    fn test_exec_block_commitment_ssz_roundtrip() {
        let commitment = ExecBlockCommitment::new([1u8; 32], [2u8; 32]);
        let encoded = commitment.as_ssz_bytes();
        let decoded = ExecBlockCommitment::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(commitment, decoded);
    }

    #[test]
    fn test_exec_block_commitment_tree_hash() {
        let commitment1 = ExecBlockCommitment::new([1u8; 32], [2u8; 32]);
        let commitment2 = ExecBlockCommitment::new([1u8; 32], [2u8; 32]);
        assert_eq!(commitment1.tree_hash_root(), commitment2.tree_hash_root());
    }

    #[test]
    fn test_subject_deposit_data_ssz_roundtrip() {
        let deposit =
            SubjectDepositData::new(SubjectId::from([3u8; 32]), BitcoinAmount::from(5000));
        let encoded = deposit.as_ssz_bytes();
        let decoded = SubjectDepositData::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(deposit, decoded);
    }

    #[test]
    fn test_subject_deposit_data_tree_hash() {
        let deposit1 =
            SubjectDepositData::new(SubjectId::from([3u8; 32]), BitcoinAmount::from(5000));
        let deposit2 =
            SubjectDepositData::new(SubjectId::from([3u8; 32]), BitcoinAmount::from(5000));
        assert_eq!(deposit1.tree_hash_root(), deposit2.tree_hash_root());
    }

    #[test]
    fn test_block_inputs_ssz_roundtrip_empty() {
        let inputs = BlockInputs::new_empty();
        let encoded = inputs.as_ssz_bytes();
        let decoded = BlockInputs::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(inputs, decoded);
        assert_eq!(decoded.total_inputs(), 0);
    }

    #[test]
    fn test_block_inputs_ssz_roundtrip_with_deposits() {
        let mut inputs = BlockInputs::new_empty();
        inputs.add_subject_deposit(SubjectDepositData::new(
            SubjectId::from([1u8; 32]),
            BitcoinAmount::from(1000),
        ));
        inputs.add_subject_deposit(SubjectDepositData::new(
            SubjectId::from([2u8; 32]),
            BitcoinAmount::from(2000),
        ));

        let encoded = inputs.as_ssz_bytes();
        let decoded = BlockInputs::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(inputs, decoded);
        assert_eq!(decoded.total_inputs(), 2);
    }

    #[test]
    fn test_output_transfer_ssz_roundtrip() {
        let transfer = OutputTransfer::new(AccountId::from([4u8; 32]), BitcoinAmount::from(3000));
        let encoded = transfer.as_ssz_bytes();
        let decoded = OutputTransfer::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(transfer, decoded);
    }

    #[test]
    fn test_block_outputs_ssz_roundtrip_empty() {
        let outputs = BlockOutputs::new_empty();
        let encoded = outputs.as_ssz_bytes();
        let decoded = BlockOutputs::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(outputs, decoded);
    }

    #[test]
    fn test_block_outputs_ssz_roundtrip_with_data() {
        use strata_acct_types::MsgPayload;

        let mut outputs = BlockOutputs::new_empty();
        outputs.add_transfer(OutputTransfer::new(
            AccountId::from([5u8; 32]),
            BitcoinAmount::from(1500),
        ));
        outputs.add_message(SentMessage::new(
            AccountId::from([6u8; 32]),
            MsgPayload::new(BitcoinAmount::from(500), vec![10, 20, 30]),
        ));

        let encoded = outputs.as_ssz_bytes();
        let decoded = BlockOutputs::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(outputs, decoded);
    }

    #[test]
    fn test_exec_block_notpackage_ssz_roundtrip() {
        use strata_acct_types::MsgPayload;

        let commitment = ExecBlockCommitment::new([7u8; 32], [8u8; 32]);

        let mut inputs = BlockInputs::new_empty();
        inputs.add_subject_deposit(SubjectDepositData::new(
            SubjectId::from([9u8; 32]),
            BitcoinAmount::from(7000),
        ));

        let mut outputs = BlockOutputs::new_empty();
        outputs.add_transfer(OutputTransfer::new(
            AccountId::from([10u8; 32]),
            BitcoinAmount::from(4000),
        ));
        outputs.add_message(SentMessage::new(
            AccountId::from([11u8; 32]),
            MsgPayload::new(BitcoinAmount::from(100), vec![1, 2, 3]),
        ));

        let block = ExecBlockNotpackage::new(commitment, inputs, outputs);
        let encoded = block.as_ssz_bytes();
        let decoded = ExecBlockNotpackage::from_ssz_bytes(&encoded).unwrap();
        assert_eq!(block, decoded);
    }

    #[test]
    fn test_exec_block_notpackage_tree_hash() {
        let commitment = ExecBlockCommitment::new([12u8; 32], [13u8; 32]);
        let inputs = BlockInputs::new_empty();
        let outputs = BlockOutputs::new_empty();

        let block1 = ExecBlockNotpackage::new(commitment, inputs.clone(), outputs.clone());
        let block2 = ExecBlockNotpackage::new(commitment, inputs, outputs);

        assert_eq!(block1.tree_hash_root(), block2.tree_hash_root());
    }
}
