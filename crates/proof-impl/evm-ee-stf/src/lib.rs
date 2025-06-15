//! EVM Execution Environment STF for Alpen prover, using RSP for EVM execution. Provides primitives
//! and utilities to process Ethereum block transactions and state transitions in a zkVM.
pub mod primitives;
pub mod program;
pub mod utils;
use std::{panic, sync::Arc};

use alloy_consensus::{BlockHeader, Header, TxReceipt};
use alpen_reth_evm::evm::AlpenEvmFactory;
pub use primitives::{EvmBlockStfInput, EvmBlockStfOutput};
use reth_chainspec::ChainSpec;
use reth_evm::execute::{BasicBlockExecutor, ExecutionOutcome, Executor};
use reth_evm_ethereum::EthEvmConfig;
use reth_primitives::EthPrimitives;
use reth_primitives_traits::block::Block;
use reth_trie::KeccakKeyHasher;
use revm::database::WrapDatabaseRef;
use revm_primitives::alloy_primitives::Bloom;
use rsp_client_executor::{
    executor::ClientExecutor, io::EthClientExecutorInput, BlockValidator, FromInput,
};
use utils::generate_exec_update;
use zkaleido::ZkVmEnv;

pub type AlpEthClientExecutor = ClientExecutor<EthEvmConfig<AlpenEvmFactory>, ChainSpec>;

pub fn process_block_transaction(mut input: EthClientExecutorInput) -> EvmBlockStfOutput {
    let chain_spec: Arc<ChainSpec> = Arc::new((&input.genesis).try_into().unwrap());
    let evm_config =
        EthEvmConfig::new_with_evm_factory(chain_spec.clone(), AlpenEvmFactory::default());
    // Initialize the witnessed database with verified storage proofs.
    let db = WrapDatabaseRef(input.witness_db().unwrap());

    let block_executor = BasicBlockExecutor::new(evm_config, db);

    let block = EthPrimitives::from_input_block(input.current_block.clone())
        .try_into_recovered()
        .expect("Failed to convert input block");

    // Validate the block header
    EthPrimitives::validate_header(block.sealed_block().sealed_header(), chain_spec.clone())
        .expect("Failed to validate block header");

    // Execute the block
    let execution_output = block_executor
        .execute(&block)
        .expect("Failed to execute block");

    // Validate the block post execution.
    EthPrimitives::validate_block_post_execution(&block, chain_spec.clone(), &execution_output)
        .expect("Failed to validate block post execution");

    // Accumulate the logs bloom.
    let mut logs_bloom = Bloom::default();
    execution_output.result.receipts.iter().for_each(|r| {
        logs_bloom.accrue_bloom(&r.bloom());
    });

    // Convert the output to an execution outcome.
    let executor_outcome = ExecutionOutcome::new(
        execution_output.state,
        vec![execution_output.result.receipts],
        input.current_block.number,
        vec![execution_output.result.requests],
    );

    // Verify the state root.
    let state_root = {
        input
            .parent_state
            .update(&executor_outcome.hash_state_slow::<KeccakKeyHasher>());
        input.parent_state.state_root()
    };

    if state_root != input.current_block.header().state_root() {
        panic!(
            "State root mismatch: expected {}, got {}",
            input.current_block.header().state_root(),
            state_root
        );
    }

    // Derive the block header.
    // Note: the receipts root and gas used are verified by `validate_block_post_execution`.
    let header = Header {
        parent_hash: input.current_block.header().parent_hash(),
        ommers_hash: input.current_block.header().ommers_hash(),
        beneficiary: input.current_block.header().beneficiary(),
        state_root,
        transactions_root: input.current_block.header().transactions_root(),
        receipts_root: input.current_block.header().receipts_root(),
        logs_bloom,
        difficulty: input.current_block.header().difficulty(),
        number: input.current_block.header().number(),
        gas_limit: input.current_block.header().gas_limit(),
        gas_used: input.current_block.header().gas_used(),
        timestamp: input.current_block.header().timestamp(),
        extra_data: input.current_block.header().extra_data().clone(),
        mix_hash: input.current_block.header().mix_hash().unwrap(),
        nonce: input.current_block.header().nonce().unwrap(),
        base_fee_per_gas: input.current_block.header().base_fee_per_gas(),
        withdrawals_root: input.current_block.header().withdrawals_root(),
        blob_gas_used: input.current_block.header().blob_gas_used(),
        excess_blob_gas: input.current_block.header().excess_blob_gas(),
        parent_beacon_block_root: input.current_block.header().parent_beacon_block_root(),
        requests_hash: input.current_block.header().requests_hash(),
    };

    EvmBlockStfOutput {
        block_idx: header.number,
        new_blockhash: header.hash_slow(),
        new_state_root: header.state_root,
        prev_blockhash: header.parent_hash,
        txn_root: header.transactions_root,
        deposit_requests: vec![],
        withdrawal_intents: Vec::new(),
    }
}

/// Processes a sequence of EL block transactions from the given `zkvm` environment, ensuring block
/// hash continuity and committing the resulting updates.
pub fn process_block_transaction_outer(zkvm: &impl ZkVmEnv) {
    let num_blocks: u32 = zkvm.read_serde();
    assert!(num_blocks > 0, "At least one block is required.");

    let mut exec_updates = Vec::with_capacity(num_blocks as usize);
    let mut current_blockhash = None;

    for _ in 0..num_blocks {
        let input: EthClientExecutorInput = zkvm.read_serde();
        let output = process_block_transaction(input);

        if let Some(expected_hash) = current_blockhash {
            assert_eq!(output.prev_blockhash, expected_hash, "Block hash mismatch");
        }

        current_blockhash = Some(output.new_blockhash);
        exec_updates.push(generate_exec_update(&output));
    }

    zkvm.commit_borsh(&exec_updates);
}

#[cfg(test)]
mod tests {

    use serde::{Deserialize, Serialize};

    use super::{process_block_transaction, EvmBlockStfInput, EvmBlockStfOutput};

    #[derive(Serialize, Deserialize)]
    struct TestData {
        witness: EvmBlockStfInput,
        params: EvmBlockStfOutput,
    }

    fn get_mock_data() -> TestData {
        let json_content = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("test_data/witness_params.json"),
        )
        .expect("Failed to read the blob data file");

        serde_json::from_str(&json_content).expect("Valid json")
    }

    #[test]
    fn basic_serde() {
        // Checks that serialization and deserialization actually works.
        let test_data = get_mock_data();

        let s = bincode::serialize(&test_data.witness).unwrap();
        let d: EvmBlockStfInput = bincode::deserialize(&s[..]).unwrap();
        assert_eq!(d, test_data.witness);
    }

    #[test]
    fn block_stf_test() {
        let test_data = get_mock_data();

        let input = test_data.witness;
        let op = process_block_transaction(input);
        assert_eq!(op, test_data.params);
    }
}
