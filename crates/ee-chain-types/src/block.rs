//! Types relating to EE block related structures with SSZ support.

use strata_acct_types::{AccountId, BitcoinAmount, Hash, SentMessage, SubjectId};

use crate::{
    BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage, OutputTransfer,
    SubjectDepositData,
};

impl ExecBlockPackage {
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

    pub fn exec_blkid(&self) -> Hash {
        self.commitment().exec_blkid()
    }

    pub fn raw_block_encoded_hash(&self) -> Hash {
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
            exec_blkid: exec_blkid.0.into(),
            raw_block_encoded_hash: raw_block_encoded_hash.0.into(),
        }
    }

    pub fn exec_blkid(&self) -> Hash {
        let mut result = [0u8; 32];
        result.copy_from_slice(self.exec_blkid.as_ref());
        Hash::new(result)
    }

    pub fn raw_block_encoded_hash(&self) -> Hash {
        let mut result = [0u8; 32];
        result.copy_from_slice(self.raw_block_encoded_hash.as_ref());
        Hash::new(result)
    }
}

impl BlockInputs {
    fn new(subject_deposits: Vec<SubjectDepositData>) -> Self {
        Self {
            subject_deposits: subject_deposits.into(),
        }
    }

    /// Creates a new empty instance.
    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn subject_deposits(&self) -> &[SubjectDepositData] {
        self.subject_deposits.as_ref()
    }

    pub fn add_subject_deposit(&mut self, d: SubjectDepositData) {
        self.subject_deposits
            .push(d)
            .expect("subject_deposits list at capacity");
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
            output_transfers: output_transfers.into(),
            output_messages: output_messages.into(),
        }
    }

    /// Creates a new empty instance.
    pub fn new_empty() -> Self {
        Self::new(Vec::new(), Vec::new())
    }

    pub fn output_transfers(&self) -> &[OutputTransfer] {
        self.output_transfers.as_ref()
    }

    /// Adds a transfer output.
    pub fn add_transfer(&mut self, t: OutputTransfer) {
        self.output_transfers
            .push(t)
            .expect("output_transfers list at capacity");
    }

    pub fn output_messages(&self) -> &[SentMessage] {
        self.output_messages.as_ref()
    }

    /// Adds a message output.
    pub fn add_message(&mut self, m: SentMessage) {
        self.output_messages
            .push(m)
            .expect("output_messages list at capacity");
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
    use proptest::prelude::*;
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;

    mod exec_block_commitment {
        use super::*;

        ssz_proptest!(
            ExecBlockCommitment,
            (any::<[u8; 32]>(), any::<[u8; 32]>()).prop_map(|(blkid, hash)| {
                ExecBlockCommitment {
                    exec_blkid: blkid.into(),
                    raw_block_encoded_hash: hash.into(),
                }
            })
        );

        #[test]
        fn test_new() {
            let blkid = Hash::new([0xaa; 32]);
            let hash = Hash::new([0xbb; 32]);
            let commitment = ExecBlockCommitment::new(blkid, hash);

            assert_eq!(commitment.exec_blkid(), blkid);
            assert_eq!(commitment.raw_block_encoded_hash(), hash);
        }
    }

    mod subject_deposit_data {
        use super::*;

        ssz_proptest!(
            SubjectDepositData,
            (any::<[u8; 32]>(), any::<u64>()).prop_map(|(dest, sats)| {
                SubjectDepositData {
                    dest: SubjectId::new(dest),
                    value: BitcoinAmount::from_sat(sats),
                }
            })
        );

        #[test]
        fn test_new() {
            let dest = SubjectId::new([0xcc; 32]);
            let value = BitcoinAmount::from_sat(1000);
            let deposit = SubjectDepositData::new(dest, value);

            assert_eq!(deposit.dest(), dest);
            assert_eq!(deposit.value(), value);
        }
    }

    mod block_inputs {
        use super::*;

        ssz_proptest!(
            BlockInputs,
            prop::collection::vec(
                (any::<[u8; 32]>(), any::<u64>()).prop_map(|(dest, sats)| {
                    SubjectDepositData {
                        dest: SubjectId::new(dest),
                        value: BitcoinAmount::from_sat(sats),
                    }
                }),
                0..10
            )
            .prop_map(|deposits| BlockInputs {
                subject_deposits: deposits.into()
            })
        );

        #[test]
        fn test_new_empty() {
            let inputs = BlockInputs::new_empty();
            assert_eq!(inputs.total_inputs(), 0);
        }

        #[test]
        fn test_add_subject_deposit() {
            let mut inputs = BlockInputs::new_empty();
            let deposit =
                SubjectDepositData::new(SubjectId::new([0xdd; 32]), BitcoinAmount::from_sat(500));

            inputs.add_subject_deposit(deposit);
            assert_eq!(inputs.total_inputs(), 1);
        }
    }

    mod output_transfer {
        use super::*;

        ssz_proptest!(
            OutputTransfer,
            (any::<[u8; 32]>(), any::<u64>()).prop_map(|(dest, sats)| {
                OutputTransfer {
                    dest: AccountId::new(dest),
                    value: BitcoinAmount::from_sat(sats),
                }
            })
        );

        #[test]
        fn test_new() {
            let dest = AccountId::new([0xee; 32]);
            let value = BitcoinAmount::from_sat(2000);
            let transfer = OutputTransfer::new(dest, value);

            assert_eq!(transfer.dest(), dest);
            assert_eq!(transfer.value(), value);
        }
    }

    mod block_outputs {
        use super::*;

        ssz_proptest!(
            BlockOutputs,
            (
                prop::collection::vec(
                    (any::<[u8; 32]>(), any::<u64>()).prop_map(|(dest, sats)| {
                        OutputTransfer {
                            dest: AccountId::new(dest),
                            value: BitcoinAmount::from_sat(sats),
                        }
                    }),
                    0..10
                ),
                prop::collection::vec(
                    (
                        any::<[u8; 32]>(),
                        any::<u64>(),
                        prop::collection::vec(any::<u8>(), 0..50)
                    )
                        .prop_map(|(dest, sats, data)| {
                            SentMessage::new(
                                AccountId::new(dest),
                                strata_acct_types::MsgPayload::new(
                                    BitcoinAmount::from_sat(sats),
                                    data,
                                ),
                            )
                        }),
                    0..10
                )
            )
                .prop_map(|(transfers, messages)| {
                    BlockOutputs {
                        output_transfers: transfers.into(),
                        output_messages: messages.into(),
                    }
                })
        );

        #[test]
        fn test_new_empty() {
            let outputs = BlockOutputs::new_empty();
            assert_eq!(outputs.output_transfers().len(), 0);
            assert_eq!(outputs.output_messages().len(), 0);
        }
    }

    mod exec_block_not_package {
        use super::*;

        #[test]
        fn test_new() {
            let commitment = ExecBlockCommitment::new(Hash::new([0xff; 32]), Hash::new([0x11; 32]));
            let inputs = BlockInputs::new_empty();
            let outputs = BlockOutputs::new_empty();

            let block = ExecBlockPackage::new(commitment, inputs, outputs);

            assert_eq!(block.exec_blkid(), Hash::new([0xff; 32]));
            assert_eq!(block.raw_block_encoded_hash(), Hash::new([0x11; 32]));
            assert_eq!(block.inputs().total_inputs(), 0);
        }
    }
}
