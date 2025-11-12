//! EVM block execution logic.
//!
//! This module provides the core ExecutionEnvironment implementation for EVM blocks,
//! using RSP's sparse state and Reth's EVM execution engine.

use std::sync::Arc;

use alloy_consensus::Header;
use alpen_reth_evm::evm::AlpenEvmFactory;
use reth_chainspec::ChainSpec;
use reth_evm::execute::{BasicBlockExecutor, Executor};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives::EthPrimitives;
use revm::database::WrapDatabaseRef;
use rsp_client_executor::BlockValidator;
use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload, SentMessage};
use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlockOutput, ExecHeader, ExecPayload, ExecutionEnvironment,
};
use strata_ee_chain_types::{BlockInputs, BlockOutputs};

use crate::{
    types::{EvmBlock, EvmHeader, EvmPartialState, EvmWriteBatch},
    utils::{
        accumulate_logs_bloom, build_and_recover_block, collect_withdrawal_intents_from_execution,
        compute_hashed_post_state,
    },
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

impl EvmExecutionEnvironment {
    /// Creates a new EvmExecutionEnvironment with the given chain specification.
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        let evm_config = EthEvmConfig::new_with_evm_factory(chain_spec, AlpenEvmFactory::default());
        Self { evm_config }
    }

    /// Converts withdrawal intents to messages sent to the bridge gateway account.
    ///
    /// Each withdrawal intent is encoded as a message containing:
    /// - The withdrawal amount (as message value)
    /// - The descriptor bytes + transaction ID (as message data)
    fn convert_withdrawal_intents_to_messages(
        withdrawal_intents: Vec<alpen_reth_primitives::WithdrawalIntent>,
        outputs: &mut BlockOutputs,
    ) {
        for intent in withdrawal_intents {
            // Encode withdrawal intent data: descriptor bytes + txid
            let mut msg_data = Vec::new();
            msg_data.extend_from_slice(&intent.destination.to_bytes());
            msg_data.extend_from_slice(&intent.withdrawal_txid.0);

            // FIXME: does this come from params.json???
            // TODO: Define the actual bridge gateway account ID
            // This should be a well-known account ID for the bridge gateway
            // that handles withdrawal intents from the EVM to L1
            let bridge_gateway_account = AccountId::from([0u8; 32]);

            // Create message to bridge gateway with withdrawal amount and intent data
            let payload = MsgPayload::new(BitcoinAmount::from_sat(intent.amt), msg_data);
            let message = SentMessage::new(bridge_gateway_account, payload);
            outputs.add_message(message);
        }
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
        // TODO: get feedbacks from Trey if this field can be unused in Eth context
        _inputs: &BlockInputs,
    ) -> EnvResult<ExecBlockOutput<Self>> {
        // Step 1: Build block from exec_payload and recover senders
        let block = build_and_recover_block(exec_payload)?;

        // Step 2: Validate header early (cheap structural consistency check)
        // This validates header fields follow consensus rules (difficulty, nonce, gas limits, etc.)
        // Cheaper checks should go before more expensive ones if they're independent
        EthPrimitives::validate_header(
            block.sealed_block().sealed_header(),
            (*self.evm_config.chain_spec()).clone(),
        )
        .map_err(|_| EnvError::InvalidBlock)?;

        // Step 3: Prepare witness database from partial state (expensive operation)
        let db = {
            let trie_db = pre_state.prepare_witness_db(exec_payload.header_intrinsics());
            WrapDatabaseRef(trie_db)
        };

        // Step 4: Create block executor
        let block_executor = BasicBlockExecutor::new(self.evm_config.clone(), db);

        // Step 5: Execute the block (expensive operation)
        let execution_output = block_executor
            .execute(&block)
            .map_err(|_| EnvError::InvalidBlock)?;

        // Step 6: Validate block post-execution
        // Note: This validates execution-dependent fields (receipts root, gas used, requests)
        // and cannot be moved to verify_outputs_against_header as it requires the full block
        // and execution_output which are not available in that context
        EthPrimitives::validate_block_post_execution(
            &block,
            (*self.evm_config.chain_spec()).clone(),
            &execution_output,
        )
        .map_err(|_| EnvError::InvalidBlock)?;

        // Step 7: Accumulate logs bloom
        let logs_bloom = accumulate_logs_bloom(&execution_output.result.receipts);

        // Step 8: Collect withdrawal intents
        let transactions = block.into_transactions();
        let withdrawal_intents =
            collect_withdrawal_intents_from_execution(transactions, &execution_output.receipts);

        // Step 9: Convert execution outcome to HashedPostState
        let block_number = exec_payload.header_intrinsics().number;
        let hashed_post_state = compute_hashed_post_state(&execution_output, block_number);

        // Step 10: Compute state root
        let state_root = pre_state.compute_state_root_with_changes(&hashed_post_state);

        // Step 11: Create WriteBatch with computed metadata
        let write_batch = EvmWriteBatch::new(hashed_post_state, state_root.into(), logs_bloom);

        // Step 12: Create BlockOutputs with withdrawal intent messages
        let mut outputs = BlockOutputs::new_empty();
        Self::convert_withdrawal_intents_to_messages(withdrawal_intents, &mut outputs);

        Ok(ExecBlockOutput::new(write_batch, outputs))
    }

    fn complete_header(
        &self,
        exec_payload: &ExecPayload<'_, Self::Block>,
        output: &ExecBlockOutput<Self>,
    ) -> EnvResult<<Self::Block as strata_ee_acct_types::ExecBlock>::Header> {
        // Complete the header using execution outputs
        // The exec_payload contains header intrinsics (non-commitment fields)

        // Get the intrinsics from the payload
        let intrinsics = exec_payload.header_intrinsics();

        // Get computed commitments from the write batch
        let state_root = output.write_batch().state_root();
        let logs_bloom = output.write_batch().logs_bloom();

        // Build the complete header with both intrinsics and computed commitments
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

    fn verify_outputs_against_header(
        &self,
        header: &<Self::Block as strata_ee_acct_types::ExecBlock>::Header,
        outputs: &ExecBlockOutput<Self>,
    ) -> EnvResult<()> {
        // Verify that the outputs match what's committed in the header

        // Check state root matches
        let computed_state_root = outputs.write_batch().state_root();
        let header_state_root = header.get_state_root();

        if computed_state_root != header_state_root {
            //FIXME: is it correct enum to represent this error?
            return Err(EnvError::MismatchedCurStateData);
        }

        // Note: transactions_root and receipts_root are verified during execution
        // by validate_block_post_execution() in execute_block_body()

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

        #[derive(Deserialize)]
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
            // Test that we can complete the header
            let completed_header = env.complete_header(&exec_payload, &output);
            assert!(completed_header.is_ok(), "Header completion should succeed");

            // Test that verification works
            if let Ok(header) = completed_header {
                let verify_result = env.verify_outputs_against_header(&header, &output);
                assert!(
                    verify_result.is_ok(),
                    "Verification should succeed with real data"
                );
            }
        }
    }
}
