// This code is modified from the original implementation of Zeth.
//
// Reference: https://github.com/risc0/zeth
//
// Copyright 2023 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either strata or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
pub mod db;
pub mod primitives;
pub mod processor;
pub mod program;
pub mod utils;
use alpen_reth_evm::collect_withdrawal_intents;
use db::InMemoryDBHelper;
pub use primitives::{EvmBlockStfInput, EvmBlockStfOutput};
use processor::{EvmConfig, EvmProcessor};
use revm::{primitives::SpecId, InMemoryDB};
use revm_primitives::alloy_primitives::B256;
use strata_mpt::keccak;
use utils::generate_exec_update;
use zkaleido::ZkVmEnv;

// TODO: Read the evm config from the genesis config. This should be done in compile time.
const EVM_CONFIG: EvmConfig = EvmConfig {
    chain_id: 2892,
    spec_id: SpecId::SHANGHAI,
};
/// Executes the block with the given input and EVM configuration, returning public parameters.
pub fn process_block_transaction(
    mut input: EvmBlockStfInput,
    evm_config: EvmConfig,
) -> EvmBlockStfOutput {
    // Calculate the previous block hash
    let previous_block_hash = B256::from(keccak(alloy_rlp::encode(input.parent_header.clone())));

    // Deposit requests are processed and forwarded as public parameters for verification on the CL
    let deposit_requests = input.withdrawals.clone();

    // Initialize the in-memory database
    let db = match InMemoryDB::initialize(&mut input) {
        Ok(database) => database,
        Err(e) => panic!("Failed to initialize database: {e:?}"),
    };

    // Create an EVM processor and execute the block
    let mut evm_processor = EvmProcessor::<InMemoryDB> {
        input,
        db: Some(db),
        header: None,
        evm_config,
    };

    evm_processor.initialize();
    let (executed_txs, receipts) = evm_processor.execute();
    evm_processor.finalize();

    // Extract the header and compute the new block hash
    let block_header = evm_processor.header.unwrap(); // Ensure header exists before unwrap
    let new_block_hash = B256::from(keccak(alloy_rlp::encode(block_header.clone())));

    // TODO: Optimize receipt iteration by implementing bloom filters or adding hints to
    // `ElBlockStfInput`. This will allow for efficient filtering of `WithdrawalIntentEvents`.
    let withdrawal_intents: Vec<_> =
        collect_withdrawal_intents(executed_txs.iter().zip(receipts.iter())).collect();

    // Construct the public parameters for the proof
    EvmBlockStfOutput {
        block_idx: block_header.number,
        new_blockhash: new_block_hash,
        new_state_root: block_header.state_root,
        prev_blockhash: previous_block_hash,
        txn_root: block_header.transactions_root,
        deposit_requests,
        withdrawal_intents,
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
        let input: EvmBlockStfInput = zkvm.read_serde();
        let output = process_block_transaction(input, EVM_CONFIG);

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
    use revm::primitives::SpecId;
    use serde::{Deserialize, Serialize};

    use super::*;
    const EVM_CONFIG: EvmConfig = EvmConfig {
        chain_id: 12345,
        spec_id: SpecId::SHANGHAI,
    };

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
        let op = process_block_transaction(input, EVM_CONFIG);
        assert_eq!(op, test_data.params);
    }
}
