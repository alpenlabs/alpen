use strata_acct_types::{AccountId, BitcoinAmount, SentMessage, SubjectId};

use crate::{ExecBlockCommitment, ExecInputs, ExecOutputs, OutputTransfer, SubjectDepositData};

impl ExecInputs {
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

impl ExecOutputs {
    fn new(output_transfers: Vec<OutputTransfer>, output_messages: Vec<SentMessage>) -> Self {
        Self {
            // TODO propagate up the bounds checks here
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
            ExecInputs,
            prop::collection::vec(
                (any::<[u8; 32]>(), any::<u64>()).prop_map(|(dest, sats)| {
                    SubjectDepositData {
                        dest: SubjectId::new(dest),
                        value: BitcoinAmount::from_sat(sats),
                    }
                }),
                0..10
            )
            .prop_map(|deposits| ExecInputs {
                subject_deposits: deposits.into()
            })
        );

        #[test]
        fn test_new_empty() {
            let inputs = ExecInputs::new_empty();
            assert_eq!(inputs.total_inputs(), 0);
        }

        #[test]
        fn test_add_subject_deposit() {
            let mut inputs = ExecInputs::new_empty();
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
            ExecOutputs,
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
                    ExecOutputs {
                        output_transfers: transfers.into(),
                        output_messages: messages.into(),
                    }
                })
        );

        #[test]
        fn test_new_empty() {
            let outputs = ExecOutputs::new_empty();
            assert_eq!(outputs.output_transfers().len(), 0);
            assert_eq!(outputs.output_messages().len(), 0);
        }
    }
}
