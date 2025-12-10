//! Commit operation types.

use strata_acct_types::Hash;
use strata_ee_chain_types::ExecBlockPackage;

use crate::{
    errors::EnvError,
    ssz_generated::ssz::commit::{CommitBlockData, CommitChainSegment},
};

impl CommitChainSegment {
    pub fn new(blocks: Vec<CommitBlockData>) -> Self {
        Self {
            blocks: blocks.into(),
        }
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

impl CommitBlockData {
    pub fn new(package: ExecBlockPackage, raw_full_block: Vec<u8>) -> Self {
        Self {
            package,
            raw_full_block: raw_full_block.into(),
        }
    }

    pub fn package(&self) -> &ExecBlockPackage {
        &self.package
    }

    pub fn raw_full_block(&self) -> &[u8] {
        &self.raw_full_block
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SubjectId};
    use strata_ee_chain_types::{
        BlockInputs, BlockOutputs, ExecBlockCommitment, ExecBlockPackage, OutputTransfer,
        SubjectDepositData,
    };
    use strata_test_utils_ssz::ssz_proptest;

    use crate::ssz_generated::ssz::commit::{CommitBlockData, CommitChainSegment};

    fn exec_block_commitment_strategy() -> impl Strategy<Value = ExecBlockCommitment> {
        (any::<[u8; 32]>(), any::<[u8; 32]>()).prop_map(|(exec_blkid, raw_hash)| {
            ExecBlockCommitment {
                exec_blkid: exec_blkid.into(),
                raw_block_encoded_hash: raw_hash.into(),
            }
        })
    }

    fn subject_deposit_data_strategy() -> impl Strategy<Value = SubjectDepositData> {
        (any::<[u8; 32]>(), any::<u64>()).prop_map(|(dest_bytes, value)| SubjectDepositData {
            dest: SubjectId::from(dest_bytes),
            value: BitcoinAmount::from_sat(value),
        })
    }

    fn block_inputs_strategy() -> impl Strategy<Value = BlockInputs> {
        prop::collection::vec(subject_deposit_data_strategy(), 0..5).prop_map(|deposits| {
            BlockInputs {
                subject_deposits: deposits.into(),
            }
        })
    }

    fn output_transfer_strategy() -> impl Strategy<Value = OutputTransfer> {
        (any::<[u8; 32]>(), any::<u64>()).prop_map(|(dest_bytes, value)| OutputTransfer {
            dest: AccountId::from(dest_bytes),
            value: BitcoinAmount::from_sat(value),
        })
    }

    fn block_outputs_strategy() -> impl Strategy<Value = BlockOutputs> {
        (
            prop::collection::vec(output_transfer_strategy(), 0..5),
            prop::collection::vec(
                (
                    any::<[u8; 32]>(),
                    any::<u64>(),
                    prop::collection::vec(any::<u8>(), 0..256),
                ),
                0..5,
            ),
        )
            .prop_map(|(transfers, messages)| {
                let sent_messages = messages
                    .into_iter()
                    .map(|(dest_bytes, value, data)| strata_acct_types::SentMessage {
                        dest: AccountId::from(dest_bytes),
                        payload: MsgPayload {
                            value: BitcoinAmount::from_sat(value),
                            data: data.into(),
                        },
                    })
                    .collect::<Vec<_>>();

                BlockOutputs {
                    output_transfers: transfers.into(),
                    output_messages: sent_messages.into(),
                }
            })
    }

    fn exec_block_package_strategy() -> impl Strategy<Value = ExecBlockPackage> {
        (
            exec_block_commitment_strategy(),
            block_inputs_strategy(),
            block_outputs_strategy(),
        )
            .prop_map(|(commitment, inputs, outputs)| ExecBlockPackage {
                commitment,
                inputs,
                outputs,
            })
    }

    mod commit_block_data {
        use super::*;

        ssz_proptest!(
            CommitBlockData,
            (
                exec_block_package_strategy(),
                prop::collection::vec(any::<u8>(), 0..1024),
            )
                .prop_map(|(package, raw_block)| CommitBlockData {
                    package,
                    raw_full_block: raw_block.into(),
                })
        );
    }

    mod commit_chain_segment {
        use super::*;

        ssz_proptest!(
            CommitChainSegment,
            prop::collection::vec(
                (
                    exec_block_package_strategy(),
                    prop::collection::vec(any::<u8>(), 0..1024),
                )
                    .prop_map(|(package, raw_block)| CommitBlockData {
                        package,
                        raw_full_block: raw_block.into(),
                    }),
                0..5
            )
            .prop_map(|blocks| CommitChainSegment {
                blocks: blocks.into(),
            })
        );
    }
}
