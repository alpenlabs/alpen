//! EVM block execution logic.
//!
//! This module provides the core ExecutionEnvironment implementation for EVM blocks,
//! using RSP's sparse state and Reth's EVM execution engine.

use std::sync::Arc;

use alpen_reth_evm::{collect_withdrawal_intents, evm::AlpenEvmFactory};
use reth_chainspec::ChainSpec;
use reth_evm::execute::{BasicBlockExecutor, ExecutionOutcome, Executor};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives::EthPrimitives;
use reth_trie::KeccakKeyHasher;
use revm::database::WrapDatabaseRef;
use rsp_client_executor::{BlockValidator, io::TrieDB};
use strata_ee_acct_types::{
    EnvError, EnvResult, ExecBlockOutput, ExecHeader, ExecPayload, ExecutionEnvironment,
};
use strata_ee_chain_types::{BlockInputs, BlockOutputs};

use crate::types::{EvmBlock, EvmPartialState, EvmWriteBatch};

/// EVM Execution Environment for Alpen.
///
/// This struct implements the ExecutionEnvironment trait and handles execution
/// of EVM blocks against sparse state using RSP and Reth.
#[derive(Debug, Clone)]
pub struct EvmExecutionEnvironment {
    /// The chain specification (genesis, fork configuration, etc.)
    chain_spec: Arc<ChainSpec>,
}

impl EvmExecutionEnvironment {
    /// Creates a new EvmExecutionEnvironment with the given chain specification.
    pub fn new(chain_spec: Arc<ChainSpec>) -> Self {
        Self { chain_spec }
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
        // Step 1: Create EVM config with AlpenEvmFactory
        let evm_config =
            EthEvmConfig::new_with_evm_factory(self.chain_spec.clone(), AlpenEvmFactory::default());

        // Step 2: Prepare data for witness DB
        // Build block_hashes map from ancestor headers (for BLOCKHASH opcode)
        // Build bytecode_by_hash map from bytecodes (for contract execution)
        use reth_primitives_traits::SealedHeader;
        use revm_primitives::{B256, map::HashMap};

        // First, seal the current block header and ancestor headers
        let current_sealed = SealedHeader::seal_slow(exec_payload.header_intrinsics().clone());
        let sealed_headers: Vec<SealedHeader> = std::iter::once(current_sealed)
            .chain(
                pre_state
                    .ancestor_headers()
                    .iter()
                    .map(|h| SealedHeader::seal_slow(h.clone())),
            )
            .collect();

        // Build block_hashes from sealed headers
        let mut block_hashes: HashMap<u64, B256> = HashMap::with_hasher(Default::default());
        for i in 0..sealed_headers.len().saturating_sub(1) {
            let child_header = &sealed_headers[i];
            let parent_header = &sealed_headers[i + 1];
            block_hashes.insert(parent_header.number, child_header.parent_hash);
        }

        // Build bytecode_by_hash from bytecodes
        let bytecode_by_hash: HashMap<B256, &revm::state::Bytecode> = pre_state
            .bytecodes()
            .iter()
            .map(|code| (code.hash_slow(), code))
            .collect();

        // Step 3: Initialize witness database from EthereumState
        let db = {
            let trie_db = TrieDB::new(pre_state.ethereum_state(), block_hashes, bytecode_by_hash);
            WrapDatabaseRef(trie_db)
        };

        // Step 4: Create block executor
        let block_executor = BasicBlockExecutor::new(evm_config, db);

        // Step 5: Build block from exec_payload and recover senders
        let header = exec_payload.header_intrinsics().clone();
        let body = exec_payload.body().body().clone();

        // Build block using alloy_consensus types
        use alloy_consensus::Block as AlloyBlock;
        let alloy_block = AlloyBlock {
            header: header.clone(),
            body,
        };

        // Recover transaction senders from signatures
        // Note: from_input_block() is just an identity function in RSP, so we skip it
        use reth_primitives_traits::Block as RethBlockTrait;
        let block = alloy_block
            .try_into_recovered()
            .map_err(|_| EnvError::InvalidBlock)?;

        // Step 6: Validate header
        EthPrimitives::validate_header(
            block.sealed_block().sealed_header(),
            self.chain_spec.clone(),
        )
        .map_err(|_| EnvError::InvalidBlock)?;

        // Step 7: Execute the block
        let execution_output = block_executor
            .execute(&block)
            .map_err(|_| EnvError::InvalidBlock)?;

        // Step 8: Validate block post-execution
        EthPrimitives::validate_block_post_execution(
            &block,
            self.chain_spec.clone(),
            &execution_output,
        )
        .map_err(|_| EnvError::InvalidBlock)?;

        // Step 9: Accumulate logs bloom
        use alloy_consensus::TxReceipt;
        use revm_primitives::alloy_primitives::Bloom;
        let mut logs_bloom = Bloom::default();
        execution_output.result.receipts.iter().for_each(|r| {
            logs_bloom.accrue_bloom(&r.bloom());
        });

        // Step 10: Collect withdrawal intents
        let transactions = block.into_transactions();
        let executed_txns = transactions.iter();
        let receipts_vec = execution_output.receipts.clone();
        let receipts = receipts_vec.iter();
        let tx_receipt_pairs = executed_txns.zip(receipts);
        let _withdrawal_intents = collect_withdrawal_intents(tx_receipt_pairs).collect::<Vec<_>>();

        // Step 11: Convert execution outcome to HashedPostState
        let block_number = header.number;
        let executor_outcome = ExecutionOutcome::new(
            execution_output.state,
            vec![execution_output.result.receipts],
            block_number,
            vec![execution_output.result.requests],
        );
        let hashed_post_state = executor_outcome.hash_state_slow::<KeccakKeyHasher>();

        // Step 12: Compute state root
        // Clone the pre-state, merge the hashed post state, and compute the new state root
        let mut updated_state = pre_state.ethereum_state().clone();
        updated_state.update(&hashed_post_state);
        let state_root = updated_state.state_root();

        // Step 13: Create WriteBatch with computed metadata
        let write_batch = EvmWriteBatch::new(hashed_post_state, state_root.into(), logs_bloom);

        // Step 14: Create BlockOutputs
        // TODO: Convert withdrawal_intents to OutputTransfer
        // WithdrawalIntent has Descriptor destination, OutputTransfer needs AccountId
        // This conversion requires business logic to map Bitcoin descriptors to AccountIds
        let outputs = BlockOutputs::new_empty();
        // for intent in withdrawal_intents {
        //     let account_id = convert_descriptor_to_account_id(&intent.destination)?;
        //     outputs.add_transfer(OutputTransfer::new(account_id, intent.amt));
        // }

        Ok(ExecBlockOutput::new(write_batch, outputs))
    }

    fn complete_header(
        &self,
        exec_payload: &ExecPayload<'_, Self::Block>,
        output: &ExecBlockOutput<Self>,
    ) -> EnvResult<<Self::Block as strata_ee_acct_types::ExecBlock>::Header> {
        // Complete the header using execution outputs
        // The exec_payload contains header intrinsics (non-commitment fields)

        use crate::types::EvmHeader;

        // Get the intrinsics from the payload
        let intrinsics = exec_payload.header_intrinsics();

        // Get computed commitments from the write batch
        let state_root = output.write_batch().state_root();
        let logs_bloom = output.write_batch().logs_bloom();

        // Build the complete header with both intrinsics and computed commitments
        use alloy_consensus::Header;
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
        // This follows the pattern from the reference process_block function from
        // proofimpl-evm-ee-stf:
        // input.parent_state.update(&hashed_post_state)
        state.ethereum_state_mut().update(wb.hashed_post_state());
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
