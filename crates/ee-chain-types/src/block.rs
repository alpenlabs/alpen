//! Types relating to EE block related structures with business logic.

// Include SSZ type definitions from ee-chain-ssz-types
include!("../../ee-chain-ssz-types/src/block.rs");

// Business logic implementations

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
