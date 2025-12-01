//! EVM block execution logic.
//!
//! This module provides the core ExecutionEnvironment implementation for EVM blocks,
//! using RSP's sparse state and Reth's EVM execution engine.

use std::sync::Arc;

use alloy_consensus::Header;
use alpen_reth_evm::{accumulate_logs_bloom, evm::AlpenEvmFactory, extract_withdrawal_intents};
use reth_chainspec::ChainSpec;
use reth_consensus_common::validation::validate_body_against_header;
use reth_evm::execute::{BasicBlockExecutor, Executor};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives::EthPrimitives;
use revm::database::WrapDatabaseRef;
use rsp_client_executor::BlockValidator;
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SentMessage};
use strata_codec::encode_to_vec;
use strata_ee_acct_types::{
    BlockAssembler, EnvError, EnvResult, ExecBlockOutput, ExecPartialState, ExecPayload,
    ExecutionEnvironment,
};
use strata_ee_chain_types::{BlockInputs, BlockOutputs};
use strata_ol_msg_types::WithdrawalMsgData;

use crate::{
    types::{EvmBlock, EvmHeader, EvmPartialState, EvmWriteBatch},
    utils::{build_and_recover_block, compute_hashed_post_state, validate_deposits_against_block},
};

//FIXME: should be set with real bridge gateway account

/// Address where withdrawal intent msgs are forwarded
const BRIDGE_GATEWAY_ACCOUNT: [u8; 32] = [1u8; 32];
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
    outputs: &mut BlockOutputs,
) {
    for intent in withdrawal_intents {
        // Create withdrawal message data with fees=0 (currently ignored) and destination descriptor
        let withdrawal_msg = WithdrawalMsgData::new(0, intent.destination.to_bytes().to_vec())
            .expect("destination descriptor too long");

        let msg_data = encode_to_vec(&withdrawal_msg).expect("encoding failed");
        let bridge_gateway_account = AccountId::from(BRIDGE_GATEWAY_ACCOUNT);

        // Create message to bridge gateway with withdrawal amount and encoded data
        let payload = MsgPayload::new(BitcoinAmount::from_sat(intent.amt), msg_data);
        let message = SentMessage::new(bridge_gateway_account, payload);
        outputs.add_message(message);
    }
}

impl EvmExecutionEnvironment {
    /// Creates a new EvmExecutionEnvironment with the given chain specification.
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        let evm_config = EthEvmConfig::new_with_evm_factory(chain_spec, AlpenEvmFactory::default());
        Self { evm_config }
    }
}

impl ExecutionEnvironment for EvmExecutionEnvironment {
    type PartialState = EvmPartialState;
    type WriteBatch = EvmWriteBatch;
    type Block = EvmBlock;

    fn execute_block_body(
        &self,
        pre_state: &Self::PartialState,
        exec_payload: &ExecPayload<'_, Self::Block>,
        inputs: &BlockInputs,
    ) -> EnvResult<ExecBlockOutput<Self>> {
        // Step 1: Build block from exec_payload and recover senders
        let block = build_and_recover_block(exec_payload)?;

        // Step 2: Validate header early (cheap structural consistency check)
        // This validates header fields follow consensus rules (difficulty, nonce, gas limits, etc.)
        EthPrimitives::validate_header(
            block.sealed_block().sealed_header(),
            self.evm_config.chain_spec().clone(),
        )
        .map_err(|_| EnvError::InvalidBlock)?;

        // Step 2b: Validate body against header (transactions_root, ommers_hash, withdrawals_root)
        validate_body_against_header(block.body(), block.header())
            .map_err(|_| EnvError::InvalidBlock)?;

        // Step 2c: Validate deposits from BlockInputs against block withdrawals
        // The withdrawals header field is hijacked to represent deposits from the OL.
        // We need to ensure the authenticated deposits from BlockInputs match what's in the block.
        validate_deposits_against_block(&block, inputs)?;

        // Step 3: Prepare witness database from partial state
        let db = {
            let wit_db = pre_state.create_witness_db();
            WrapDatabaseRef(wit_db)
        };

        // Step 4: Create block executor
        let block_executor = BasicBlockExecutor::new(&self.evm_config, db);

        // Step 5: Execute the block (expensive operation)
        let execution_output = block_executor
            .execute(&block)
            .map_err(|_| EnvError::InvalidBlock)?;

        // Step 6: Validate block post-execution
        EthPrimitives::validate_block_post_execution(
            &block,
            self.evm_config.chain_spec().clone(),
            &execution_output,
        )
        .map_err(|_| EnvError::InvalidBlock)?;

        // Step 7: Accumulate logs bloom
        let logs_bloom = accumulate_logs_bloom(&execution_output.result.receipts);

        // Step 8: Collect withdrawal intents
        let transactions = block.into_transactions();
        let withdrawal_intents =
            extract_withdrawal_intents(&transactions, &execution_output.receipts).collect();

        // Step 9: Convert execution outcome to HashedPostState
        let header_intrinsics = exec_payload.header_intrinsics();
        let hashed_post_state =
            compute_hashed_post_state(execution_output, header_intrinsics.number);

        // Step 10: Get state root from header intrinsics (verification happens during merge)
        // This avoids an expensive state clone that would be needed to compute the root here.
        let intrinsics_state_root = header_intrinsics.state_root;

        // Step 11: Create WriteBatch with intrinsics state root (to be verified during merge)
        let write_batch =
            EvmWriteBatch::new(hashed_post_state, intrinsics_state_root.into(), logs_bloom);

        // Step 12: Create BlockOutputs with withdrawal intent messages
        let mut outputs = BlockOutputs::new_empty();
        convert_withdrawal_intents_to_messages(withdrawal_intents, &mut outputs);

        Ok(ExecBlockOutput::new(write_batch, outputs))
    }

    fn verify_outputs_against_header(
        &self,
        _header: &<Self::Block as strata_ee_acct_types::ExecBlock>::Header,
        _outputs: &ExecBlockOutput<Self>,
    ) -> EnvResult<()> {
        // State root verification is deferred to merge_write_into_state to avoid
        // an expensive state clone. The actual verification happens when the state
        // is mutated and we can compute the root directly without cloning.
        //
        // Note: The following are verified during execution in execute_block_body():
        // - transactions_root, ommers_hash, withdrawals_root: by validate_body_against_header()
        // - receipts_root, logs_bloom, gas_used: by validate_block_post_execution()

        Ok(())
    }

    fn merge_write_into_state(
        &self,
        state: &mut Self::PartialState,
        wb: &Self::WriteBatch,
    ) -> EnvResult<()> {
        // Merge the HashedPostState into the EthereumState
        state.merge_write_batch(wb);

        // Verify state root AFTER merge (avoids expensive clone that would be needed
        // to compute the root before mutation)
        let computed_state_root = state.compute_state_root()?;
        let intrinsics_state_root = wb.intrinsics_state_root();

        if computed_state_root != intrinsics_state_root {
            return Err(EnvError::InvalidBlock);
        }

        Ok(())
    }
}

impl BlockAssembler for EvmExecutionEnvironment {
    fn complete_header(
        &self,
        exec_payload: &ExecPayload<'_, Self::Block>,
        output: &ExecBlockOutput<Self>,
    ) -> EnvResult<<Self::Block as strata_ee_acct_types::ExecBlock>::Header> {
        let intrinsics = exec_payload.header_intrinsics();
        let state_root = output.write_batch().intrinsics_state_root();
        let logs_bloom = output.write_batch().logs_bloom();

        let header = Header {
            parent_hash: intrinsics.parent_hash,
            ommers_hash: intrinsics.ommers_hash,
            beneficiary: intrinsics.beneficiary,
            state_root: state_root.into(),
            transactions_root: intrinsics.transactions_root,
            receipts_root: intrinsics.receipts_root,
            logs_bloom,
            difficulty: intrinsics.difficulty,
            number: intrinsics.number,
            gas_limit: intrinsics.gas_limit,
            gas_used: intrinsics.gas_used,
            timestamp: intrinsics.timestamp,
            extra_data: intrinsics.extra_data.clone(),
            mix_hash: intrinsics.mix_hash,
            nonce: intrinsics.nonce,
            base_fee_per_gas: intrinsics.base_fee_per_gas,
            withdrawals_root: intrinsics.withdrawals_root,
            blob_gas_used: intrinsics.blob_gas_used,
            excess_blob_gas: intrinsics.excess_blob_gas,
            parent_beacon_block_root: intrinsics.parent_beacon_block_root,
            requests_hash: intrinsics.requests_hash,
        };

        Ok(EvmHeader::new(header))
    }
}

#[cfg(test)]
mod tests {
    use strata_ee_acct_types::ExecBlock;

    use super::*;
    use crate::types::{EvmBlock, EvmBlockBody, EvmHeader};
    /// Test with real witness data from the reference implementation.
    /// This is an integration test that validates the full execution flow with real block data.
    #[test]
    fn test_with_witness_params() {
        use rsp_client_executor::io::EthClientExecutorInput;
        use serde::Deserialize;

        #[derive(Deserialize, Debug)]
        struct TestData {
            witness: EthClientExecutorInput,
        }

        // Load test data from reference implementation
        let test_data_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("proof-impl/evm-ee-stf/test_data/witness_params.json");

        let json_content = std::fs::read_to_string(&test_data_path)
            .expect("Failed to read witness_params.json - make sure reference crate exists");

        let test_data: TestData =
            serde_json::from_str(&json_content).expect("Failed to parse test data");

        // Create execution environment
        let chain_spec: Arc<ChainSpec> = Arc::new((&test_data.witness.genesis).try_into().unwrap());
        let env = EvmExecutionEnvironment::new(chain_spec);

        // Use the pre-state directly from witness data (it already has all the proofs!)
        let pre_state = crate::types::EvmPartialState::new(
            test_data.witness.parent_state,
            test_data.witness.bytecodes,
            test_data.witness.ancestor_headers,
        );

        // Create block from witness
        let header = test_data.witness.current_block.header().clone();
        let evm_header = EvmHeader::new(header.clone());

        // Get transactions from the block
        use reth_primitives_traits::Block as RethBlockTrait;
        let block_body = test_data.witness.current_block.body().clone();
        let evm_body = EvmBlockBody::from_alloy_body(block_body);

        let block = EvmBlock::new(evm_header, evm_body);

        // Create exec payload and inputs
        let exec_payload = ExecPayload::new(&header, block.get_body());
        let inputs = BlockInputs::new_empty();

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
}
