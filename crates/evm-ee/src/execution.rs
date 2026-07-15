//! EVM block execution logic.
//!
//! This module provides the core ExecutionEnvironment implementation for EVM blocks,
//! using RSP's sparse state and Reth's EVM execution engine.

use std::sync::Arc;

use alloy_consensus::Block as AlloyBlock;
use alpen_reth_evm::{evm::AlpenEvmFactory, extract_withdrawal_intents};
use reth_chainspec::ChainSpec;
use reth_consensus_common::validation::validate_body_against_header;
use reth_evm::{
    ConfigureEvm,
    execute::{BasicBlockExecutor, BlockExecutionOutput, Executor},
};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives::{
    EthPrimitives, Receipt as EthereumReceipt, RecoveredBlock, TransactionSigned,
};
use revm::database::WrapDatabaseRef;
use rsp_client_executor::BlockValidator;
use strata_acct_types::{BRIDGE_GATEWAY_ACCT_ID, BitcoinAmount, MsgPayload};
use strata_codec::encode_to_vec;
use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlock, ExecBlockOutput, ExecPayload, ExecutionEnvironment,
};
use strata_ee_chain_types::{ExecInputs, ExecOutputs, OutputMessage};
use strata_msg_fmt::{Msg as MsgTrait, OwnedMsg};
use strata_ol_msg_types::{DEFAULT_OPERATOR_FEE, WITHDRAWAL_MSG_TYPE_ID, WithdrawalMsgData};

use crate::{
    types::{EvmBlock, EvmBlockOutput, EvmPartialState, EvmWriteBatch},
    utils::{build_and_recover_block, compute_hashed_post_state, validate_deposits_against_block},
};

/// EVM Execution Environment for Alpen.
///
/// This struct implements the ExecutionEnvironment trait and handles execution
/// of EVM blocks against sparse state using RSP and Reth.
#[derive(Debug, Clone)]
pub struct EvmExecutionEnvironment {
    /// EVM configuration with AlpenEvmFactory (contains chain spec)
    evm_config: EthEvmConfig<ChainSpec, AlpenEvmFactory>,
}

/// Converts withdrawal intents to messages sent to the bridge gateway account.
///
/// Each withdrawal intent is encoded using `WithdrawalMsgData` containing:
/// - The withdrawal amount (as message value)
/// - The destination descriptor (encoded in message data)
fn convert_withdrawal_intents_to_messages(
    withdrawal_intents: Vec<alpen_reth_primitives::WithdrawalIntent>,
    outputs: &mut ExecOutputs,
) {
    for intent in withdrawal_intents {
        let withdrawal_msg = WithdrawalMsgData::new(
            DEFAULT_OPERATOR_FEE,
            intent.destination.to_bytes().to_vec(),
            intent.selected_operator.raw(),
        )
        .expect("invalid withdrawal destination descriptor");

        let msg_body = encode_to_vec(&withdrawal_msg).expect("encoding failed");
        let msg = OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, msg_body).expect("create message");
        let msg_data = msg.to_vec();

        // Create message to bridge gateway with withdrawal amount and encoded data
        let payload = MsgPayload::from_bytes(BitcoinAmount::from_sat(intent.amt), msg_data)
            .expect("withdrawal message payload bytes must fit within SSZ max length");
        let message = OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, payload);
        outputs.add_message(message);
    }
}

impl EvmExecutionEnvironment {
    /// Creates a new EvmExecutionEnvironment with the given chain specification
    /// and EVM factory.
    pub fn new(chain_spec: Arc<ChainSpec>, evm_factory: AlpenEvmFactory) -> Self {
        let evm_config = EthEvmConfig::new_with_evm_factory(chain_spec, evm_factory);
        Self { evm_config }
    }

    fn validate_execution_inputs(
        &self,
        block: &RecoveredBlock<AlloyBlock<TransactionSigned>>,
        inputs: &ExecInputs,
    ) -> EnvResult<()> {
        EthPrimitives::validate_header(
            block.sealed_block().sealed_header(),
            self.evm_config.chain_spec().clone(),
        )
        .map_err(|_| EnvError::InvalidBlock)?;
        validate_body_against_header(block.body(), block.header())
            .map_err(|_| EnvError::InvalidBlock)?;
        validate_deposits_against_block(block, inputs)
    }

    fn execute_recovered_block(
        &self,
        block: &RecoveredBlock<AlloyBlock<TransactionSigned>>,
        pre_state: &EvmPartialState,
    ) -> EnvResult<BlockExecutionOutput<EthereumReceipt>> {
        let db = {
            let wit_db = pre_state.create_witness_db();
            WrapDatabaseRef(wit_db)
        };
        let block_executor = BasicBlockExecutor::new(&self.evm_config, db);
        block_executor
            .execute(block)
            .map_err(|_| EnvError::InvalidBlock)
    }
}

impl ExecutionEnvironment for EvmExecutionEnvironment {
    type PartialState = EvmPartialState;
    type WriteBatch = EvmWriteBatch;
    type BlockOutput = EvmBlockOutput;
    type Block = EvmBlock;

    fn execute_block_body(
        &self,
        pre_state: &Self::PartialState,
        exec_payload: &ExecPayload<'_, Self::Block>,
        inputs: &ExecInputs,
    ) -> EnvResult<ExecBlockOutput<Self>> {
        // Step 1: Build block from exec_payload and recover senders
        let block = build_and_recover_block(exec_payload)?;

        // Step 2: Validate execution inputs against the synthesized execution header.
        // The full block header is checked separately by `verify_outputs_against_header`.
        self.validate_execution_inputs(&block, inputs)?;

        // Step 3: Execute the block.
        let execution_output = self.execute_recovered_block(&block, pre_state)?;

        // Step 4: Accumulate execution commitments.
        let header_intrinsics = exec_payload.header_intrinsics();
        let block_output =
            EvmBlockOutput::from_header_and_output(header_intrinsics, &execution_output);

        // Step 5: Collect withdrawal intents.
        let transactions = block.into_transactions();
        let withdrawal_intents = extract_withdrawal_intents(
            &transactions,
            &execution_output.receipts,
            self.evm_config.evm_factory().bridge_params(),
        )
        .map_err(|_| EnvError::InvalidBlock)?;

        // Step 6: Convert execution outcome to HashedPostState.
        let block_number = header_intrinsics.number();
        let hashed_post_state = compute_hashed_post_state(execution_output, block_number);

        // Step 7: Split state writes from execution-derived header commitments.
        let write_batch = EvmWriteBatch::new(hashed_post_state);

        // Step 8: Create ExecOutputs with withdrawal intent messages.
        let mut outputs = ExecOutputs::new_empty();
        convert_withdrawal_intents_to_messages(withdrawal_intents, &mut outputs);

        Ok(ExecBlockOutput::new(write_batch, block_output, outputs))
    }

    fn verify_outputs_against_header(
        &self,
        header: &<Self::Block as strata_ee_acct_types::ExecBlock>::Header,
        outputs: &ExecBlockOutput<Self>,
    ) -> EnvResult<()> {
        let header = header.header();
        let block_output = outputs.block_output();

        if header.gas_used > header.gas_limit
            || block_output.receipts_root() != header.receipts_root
            || block_output.logs_bloom() != header.logs_bloom
            || block_output.gas_used() != header.gas_used
            || block_output.blob_gas_used() != header.blob_gas_used
            || block_output.requests_hash() != header.requests_hash
        {
            return Err(EnvError::InvalidBlock);
        }

        Ok(())
    }

    fn merge_write_into_state(
        &self,
        state: &mut Self::PartialState,
        wb: &Self::WriteBatch,
    ) -> EnvResult<()> {
        // Merge the HashedPostState into the EthereumState
        state.merge_write_batch(wb);

        Ok(())
    }

    fn update_partial_state_after_block(
        &self,
        state: &mut Self::PartialState,
        header: &<Self::Block as ExecBlock>::Header,
    ) -> EnvResult<()> {
        state.add_executed_block(header.header().clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::PathBuf};

    use alloy_consensus::Sealable;
    use reth_primitives_traits::Block as RethBlockTrait;
    use revm::{DatabaseRef, state::Bytecode};
    use revm_primitives::{B256, alloy_primitives::Bloom};
    use rsp_client_executor::io::EthClientExecutorInput;
    use serde::Deserialize;
    use strata_ee_acct_types::{ExecBlock, ExecHeader};
    use strata_msg_fmt::{Msg, MsgRef};
    use strata_ol_bridge_types::OperatorSelection;
    use strata_ol_msg_types::OLMessageExt;
    use strata_primitives::bitcoin_bosd::Descriptor;

    use super::*;
    use crate::types::{EvmBlock, EvmBlockBody, EvmHeader, EvmPartialState};

    fn rehashed_fixture_bytecodes(bytecodes: Vec<Bytecode>) -> BTreeMap<B256, Bytecode> {
        // The RSP fixture stores bytecodes as a Vec without the original code-hash
        // keys. Re-hashing here preserves the old fixture behavior; production
        // range witnesses pass keyed bytecodes from AccessedStateGenerator.
        bytecodes
            .into_iter()
            .map(|bytecode| (bytecode.hash_slow(), bytecode))
            .collect()
    }

    #[test]
    fn withdrawal_messages_are_sent_to_bridge_gateway_with_msg_envelope() {
        let mut destination_bytes = vec![0x03];
        destination_bytes.extend_from_slice(&[0x22; 20]);
        let destination =
            Descriptor::from_bytes(&destination_bytes).expect("valid p2wpkh descriptor");
        let withdrawal_sats = 1_000_000_000;

        let intent = alpen_reth_primitives::WithdrawalIntent {
            amt: withdrawal_sats,
            selected_operator: OperatorSelection::any(),
            destination,
        };

        let mut outputs = ExecOutputs::new_empty();
        convert_withdrawal_intents_to_messages(vec![intent], &mut outputs);

        let [message] = outputs.output_messages() else {
            panic!("expected exactly one withdrawal output message");
        };
        assert_eq!(message.dest(), BRIDGE_GATEWAY_ACCT_ID);
        assert_eq!(
            message.payload().value(),
            BitcoinAmount::from_sat(withdrawal_sats)
        );

        let msg = MsgRef::try_from(message.payload().data()).expect("message envelope");
        assert_eq!(msg.ty(), WITHDRAWAL_MSG_TYPE_ID);

        let withdrawal = msg.try_as_withdrawal().expect("withdrawal payload");
        assert_eq!(withdrawal.fees(), DEFAULT_OPERATOR_FEE);
        assert_eq!(
            withdrawal.selected_operator(),
            OperatorSelection::any().raw()
        );
        assert_eq!(withdrawal.dest_desc(), destination_bytes.as_slice());
    }

    #[test]
    fn update_partial_state_after_block_adds_block_hash_for_subsequent_blocks() {
        #[derive(Deserialize, Debug)]
        struct TestData {
            witness: EthClientExecutorInput,
        }

        let test_data_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("test-utils/data/evm_ee/witness_params.json");

        let json_content = fs::read_to_string(&test_data_path)
            .expect("Failed to read witness_params.json from test-utils/data/evm_ee");

        let test_data: TestData =
            serde_json::from_str(&json_content).expect("Failed to parse test data");

        let chain_spec: Arc<ChainSpec> = Arc::new((&test_data.witness.genesis).try_into().unwrap());
        let env = EvmExecutionEnvironment::new(
            chain_spec,
            AlpenEvmFactory::from_bridge_params(&strata_bridge_params::BridgeParams::default()),
        );
        let header = test_data.witness.current_block.header().clone();
        let evm_header = EvmHeader::new(header.clone());
        let mut state = EvmPartialState::new(
            test_data.witness.parent_state,
            rehashed_fixture_bytecodes(test_data.witness.bytecodes),
            test_data.witness.ancestor_headers,
        );

        assert_eq!(
            state
                .create_witness_db()
                .block_hash_ref(header.number)
                .expect("block hash lookup must succeed"),
            B256::ZERO
        );

        env.update_partial_state_after_block(&mut state, &evm_header)
            .expect("post-block state update should succeed");

        assert_eq!(
            state
                .create_witness_db()
                .block_hash_ref(header.number)
                .expect("block hash lookup must succeed"),
            header.seal_slow().hash()
        );
    }

    /// Test with real witness data from the reference implementation.
    /// This is an integration test that validates the full execution flow with real block data.
    #[test]
    fn test_with_witness_params() {
        #[derive(Deserialize, Debug)]
        struct TestData {
            witness: EthClientExecutorInput,
        }

        // Load test data from the canonical fixture under test-utils/data/.
        let test_data_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("test-utils/data/evm_ee/witness_params.json");

        let json_content = fs::read_to_string(&test_data_path)
            .expect("Failed to read witness_params.json from test-utils/data/evm_ee");

        let test_data: TestData =
            serde_json::from_str(&json_content).expect("Failed to parse test data");

        // Create execution environment
        let chain_spec: Arc<ChainSpec> = Arc::new((&test_data.witness.genesis).try_into().unwrap());
        let env = EvmExecutionEnvironment::new(
            chain_spec,
            AlpenEvmFactory::from_bridge_params(&strata_bridge_params::BridgeParams::default()),
        );

        // Use the pre-state directly from witness data (it already has all the proofs!)
        let pre_state = EvmPartialState::new(
            test_data.witness.parent_state,
            rehashed_fixture_bytecodes(test_data.witness.bytecodes),
            test_data.witness.ancestor_headers,
        );

        // Create block from witness
        let header = test_data.witness.current_block.header().clone();
        let evm_header = EvmHeader::new(header.clone());

        // Get transactions from the block
        let block_body = test_data.witness.current_block.body().clone();
        let evm_body = EvmBlockBody::from_alloy_body(block_body);

        let block = EvmBlock::new(evm_header, evm_body);

        // Create exec payload and inputs
        let intrinsics = block.get_header().get_intrinsics();
        let exec_payload = ExecPayload::new(&intrinsics, block.get_body());
        let inputs = ExecInputs::new_empty();

        // Execute the block
        // Note: This will execute real block data through our implementation
        let result = env.execute_block_body(&pre_state, &exec_payload, &inputs);

        // For now, we just verify it doesn't panic
        // In the future, we can compare outputs with the reference implementation
        assert!(
            result.is_ok(),
            "Block execution should succeed with witness data: {:?}",
            result.err()
        );

        if let Ok(output) = result {
            // Test that verification works against the original witness header
            // This validates our computed outputs match the expected results from the witness data
            let verify_result = env.verify_outputs_against_header(block.get_header(), &output);
            assert!(
                verify_result.is_ok(),
                "Verification should succeed: our computed state_root should match witness header"
            );
        }
    }

    #[test]
    fn verify_outputs_rejects_mismatched_non_state_commitments() {
        #[derive(Deserialize, Debug)]
        struct TestData {
            witness: EthClientExecutorInput,
        }

        let test_data_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("test-utils/data/evm_ee/witness_params.json");

        let json_content = fs::read_to_string(&test_data_path)
            .expect("Failed to read witness_params.json from test-utils/data/evm_ee");

        let test_data: TestData =
            serde_json::from_str(&json_content).expect("Failed to parse test data");

        let chain_spec: Arc<ChainSpec> = Arc::new((&test_data.witness.genesis).try_into().unwrap());
        let env = EvmExecutionEnvironment::new(
            chain_spec,
            AlpenEvmFactory::from_bridge_params(&strata_bridge_params::BridgeParams::default()),
        );
        let pre_state = EvmPartialState::new(
            test_data.witness.parent_state,
            rehashed_fixture_bytecodes(test_data.witness.bytecodes),
            test_data.witness.ancestor_headers,
        );

        let header = test_data.witness.current_block.header().clone();
        let evm_header = EvmHeader::new(header.clone());
        let block_body = test_data.witness.current_block.body().clone();
        let evm_body = EvmBlockBody::from_alloy_body(block_body);
        let block = EvmBlock::new(evm_header, evm_body);
        let intrinsics = block.get_header().get_intrinsics();
        let exec_payload = ExecPayload::new(&intrinsics, block.get_body());
        let inputs = ExecInputs::new_empty();
        let output = env
            .execute_block_body(&pre_state, &exec_payload, &inputs)
            .expect("block execution should succeed");

        let mut bad_receipts_root = header.clone();
        bad_receipts_root.receipts_root = B256::from([0x11; 32]);
        assert!(
            env.verify_outputs_against_header(&EvmHeader::new(bad_receipts_root), &output)
                .is_err()
        );

        let mut bad_logs_bloom = header.clone();
        bad_logs_bloom.logs_bloom = Bloom::from([0x22; 256]);
        assert!(
            env.verify_outputs_against_header(&EvmHeader::new(bad_logs_bloom), &output)
                .is_err()
        );

        let mut bad_gas_used = header;
        bad_gas_used.gas_used = bad_gas_used.gas_used.saturating_add(1);
        assert!(
            env.verify_outputs_against_header(&EvmHeader::new(bad_gas_used), &output)
                .is_err()
        );
    }
}
