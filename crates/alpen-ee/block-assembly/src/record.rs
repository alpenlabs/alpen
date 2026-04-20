//! Assembly of a single exec block into a persistable [`ExecBlockRecord`].

use std::num::NonZero;

use alpen_ee_common::{ExecBlockPayload, ExecBlockRecord, PayloadBuilderEngine};
use eyre::Context;
use strata_acct_types::{AccountId, Hash, MessageEntry};
use strata_identifiers::OLBlockCommitment;

use crate::{build_next_exec_block, BlockAssemblyInputs, BlockAssemblyOutputs};

/// Inputs required to assemble the next [`ExecBlockRecord`].
///
/// Callers gather these from the appropriate sources (sequencer: block-builder
/// timer loop + `OLChainTracker`; fullnode: Reth ExEx notification +
/// `OLInboxClient`) and pass them in uniformly.
#[derive(Debug)]
pub struct AssembleExecBlockInputs<'a> {
    /// The parent block's record. The next block's account state, blocknum,
    /// and parent-hash are derived from this.
    pub parent_record: &'a ExecBlockRecord,
    /// New inbox messages to include in this block. Empty if no new OL inputs
    /// are being consumed.
    pub inbox_messages: Vec<MessageEntry>,
    /// Next inbox message index after consuming `inbox_messages`.
    pub next_inbox_msg_idx: u64,
    /// OL block commitment that this exec block builds on top of.
    pub best_ol_block: OLBlockCommitment,
    /// Timestamp of the new block in milliseconds since epoch.
    pub timestamp_ms: u64,
    /// Max deposits processed per block.
    pub max_deposits_per_block: NonZero<u8>,
    /// Bridge gateway account id on OL.
    pub bridge_gateway_account_id: AccountId,
}

/// Output of assembling the next [`ExecBlockRecord`].
///
/// The caller is responsible for persisting the record (`ExecBlockStorage::save_exec_block`)
/// and, on the sequencer, submitting the payload back to the engine. The
/// `blockhash` is the id of the new block and is provided separately for
/// convenience.
#[derive(Debug)]
pub struct AssembledExecBlock {
    /// Fully-built block record, ready to persist.
    pub record: ExecBlockRecord,
    /// Serialized payload bytes matching `record`'s exec package.
    pub payload: ExecBlockPayload,
    /// Blockhash of the newly assembled block (== `record.blockhash()`).
    pub blockhash: Hash,
}

/// Assembles the next exec block from `inputs` using `payload_builder`.
///
/// This is the deterministic core shared between the sequencer's block-builder
/// task and the fullnode's re-execution ExEx. It does NOT persist — callers
/// must call [`alpen_ee_common::ExecBlockStorage::save_exec_block`] on the
/// returned record / payload pair, plus any node-specific steps (sequencer:
/// `submit_payload`, both: notify the exec-chain tracker).
pub async fn assemble_next_exec_block_record<E: PayloadBuilderEngine>(
    inputs: AssembleExecBlockInputs<'_>,
    payload_builder: &E,
) -> eyre::Result<AssembledExecBlock> {
    let AssembleExecBlockInputs {
        parent_record,
        inbox_messages,
        next_inbox_msg_idx,
        best_ol_block,
        timestamp_ms,
        max_deposits_per_block,
        bridge_gateway_account_id,
    } = inputs;

    let assembly_inputs = BlockAssemblyInputs {
        account_state: parent_record.account_state().clone(),
        inbox_messages: &inbox_messages,
        parent_exec_blkid: parent_record.package().exec_blkid(),
        timestamp_ms,
        max_deposits_per_block,
        bridge_gateway_account_id,
    };

    let BlockAssemblyOutputs {
        package,
        payload,
        account_state,
    } = build_next_exec_block(assembly_inputs, payload_builder)
        .await
        .context("assemble_next_exec_block_record: failed to build exec block")?;

    let blockhash = package.exec_blkid();
    let parent_blockhash = parent_record.package().exec_blkid();

    let record = ExecBlockRecord::new(
        package,
        account_state,
        parent_record.blocknum() + 1,
        best_ol_block,
        timestamp_ms,
        parent_blockhash,
        next_inbox_msg_idx,
        inbox_messages,
    );

    Ok(AssembledExecBlock {
        record,
        payload,
        blockhash,
    })
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, sync::Mutex};

    use alloy_rpc_types_engine::ForkchoiceState;
    use alpen_ee_common::{ExecutionEngine, ExecutionEngineError, PayloadBuildAttributes};
    use alpen_reth_node::WithdrawalIntent;
    use async_trait::async_trait;
    use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
    use strata_ee_acct_types::EeAccountState;
    use strata_ee_chain_types::{ExecBlockCommitment, ExecBlockPackage, ExecInputs, ExecOutputs};
    use strata_identifiers::{Buf32, OLBlockId};

    use super::*;

    /// Payload produced by [`MockPayloadBuilder`]. Carries the blockhash it
    /// wants the assembly to see for the built block.
    #[derive(Debug, Clone)]
    struct MockPayload {
        blockhash: Hash,
    }

    impl alpen_ee_common::EnginePayload for MockPayload {
        type Error = Infallible;

        fn blocknum(&self) -> u64 {
            0
        }

        fn blockhash(&self) -> Hash {
            self.blockhash
        }

        fn withdrawal_intents(&self) -> &[WithdrawalIntent] {
            &[]
        }

        fn to_bytes(&self) -> Result<Vec<u8>, Self::Error> {
            Ok(self.blockhash.as_ref().to_vec())
        }

        fn from_bytes(bytes: &[u8]) -> Result<Self, Self::Error> {
            let mut hash = [0u8; 32];
            let len = bytes.len().min(32);
            hash[..len].copy_from_slice(&bytes[..len]);
            Ok(Self {
                blockhash: Hash::from(hash),
            })
        }
    }

    /// Mock payload builder that returns a preconfigured blockhash without
    /// actually running the EVM.
    #[derive(Default)]
    struct MockPayloadBuilder {
        built_blockhash: Mutex<Hash>,
    }

    impl MockPayloadBuilder {
        fn new(built_blockhash: Hash) -> Self {
            Self {
                built_blockhash: Mutex::new(built_blockhash),
            }
        }
    }

    #[async_trait]
    impl ExecutionEngine for MockPayloadBuilder {
        type TEnginePayload = MockPayload;

        async fn submit_payload(
            &self,
            _payload: Self::TEnginePayload,
        ) -> Result<(), ExecutionEngineError> {
            Ok(())
        }

        async fn update_consensus_state(
            &self,
            _state: ForkchoiceState,
        ) -> Result<(), ExecutionEngineError> {
            Ok(())
        }
    }

    #[async_trait]
    impl PayloadBuilderEngine for MockPayloadBuilder {
        async fn build_payload(
            &self,
            _build_attrs: PayloadBuildAttributes,
        ) -> eyre::Result<Self::TEnginePayload> {
            Ok(MockPayload {
                blockhash: *self.built_blockhash.lock().unwrap(),
            })
        }
    }

    fn make_parent_record(blocknum: u64, timestamp_ms: u64) -> ExecBlockRecord {
        let hash = Hash::from(Buf32::new([blocknum as u8; 32]));
        let package = ExecBlockPackage::new(
            ExecBlockCommitment::new(hash, hash),
            ExecInputs::new_empty(),
            ExecOutputs::new_empty(),
        );
        let account_state = EeAccountState::new(hash, BitcoinAmount::ZERO, vec![], vec![]);
        let ol_block = OLBlockCommitment::new(0, OLBlockId::from(Buf32::new([0u8; 32])));
        ExecBlockRecord::new(
            package,
            account_state,
            blocknum,
            ol_block,
            timestamp_ms,
            Hash::default(),
            0,
            vec![],
        )
    }

    fn make_message(source_byte: u8, value_sats: u64) -> MessageEntry {
        MessageEntry::new(
            AccountId::new([source_byte; 32]),
            0,
            MsgPayload::new(BitcoinAmount::from_sat(value_sats), vec![]),
        )
    }

    #[tokio::test]
    async fn preserves_inbox_message_order_into_record() {
        // The assembly helper must pass inbox messages through verbatim into
        // the resulting ExecBlockRecord — order and content preserved — so the
        // record matches what the OL side included. Regression guard: prior
        // refactors routed this through a separate helper; we need the same
        // guarantee through the new shared `assemble_next_exec_block_record`.
        let parent_record = make_parent_record(5, 5_000);
        let ol_block = OLBlockCommitment::new(10, OLBlockId::from(Buf32::new([0xaa; 32])));
        let msg1 = make_message(1, 100);
        let msg2 = make_message(2, 200);
        let messages = vec![msg1.clone(), msg2.clone()];
        let built_hash = Hash::from(Buf32::new([0xbb; 32]));
        let payload_builder = MockPayloadBuilder::new(built_hash);

        let inputs = AssembleExecBlockInputs {
            parent_record: &parent_record,
            inbox_messages: messages.clone(),
            next_inbox_msg_idx: 7,
            best_ol_block: ol_block,
            timestamp_ms: 6_000,
            max_deposits_per_block: NonZero::new(10).unwrap(),
            bridge_gateway_account_id: AccountId::new([0u8; 32]),
        };

        let assembled = assemble_next_exec_block_record(inputs, &payload_builder)
            .await
            .expect("assembly succeeds");

        let record_messages = assembled.record.messages();
        assert_eq!(record_messages.len(), 2);
        assert_eq!(record_messages[0].source(), msg1.source());
        assert_eq!(record_messages[1].source(), msg2.source());
        assert_eq!(assembled.record.blocknum(), parent_record.blocknum() + 1);
        assert_eq!(
            assembled.record.parent_blockhash(),
            parent_record.package().exec_blkid()
        );
        assert_eq!(assembled.record.timestamp_ms(), 6_000);
        assert_eq!(assembled.record.next_inbox_msg_idx(), 7);
        assert_eq!(assembled.record.ol_block(), &ol_block);
    }
}
