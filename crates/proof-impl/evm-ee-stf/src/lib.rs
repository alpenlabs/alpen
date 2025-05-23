//! This code is modified from the original implementation of Zeth.
//!
//! Reference: <https://github.com/risc0/zeth>
//!
//! Copyright 2023 RISC Zero, Inc.
//!
//! Licensed under the Apache License, Version 2.0 (the "License");
//! you may not use this file except in compliance with the License.
//! You may obtain a copy of the License at
//!
//! <http://www.apache.org/licenses/LICENSE-2.0>
//!
//! Unless required by applicable law or agreed to in writing, software
//! distributed under the License is distributed on an "AS IS" BASIS,
//! WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either strata or implied.
//! See the License for the specific language governing permissions and
//! limitations under the License.
pub mod db;
pub mod mpt;
pub mod primitives;
// pub mod processor;
pub mod program;
pub mod utils;
use db::InMemoryDBHelper;
use mpt::keccak;
pub use primitives::{EvmBlockStfInput, EvmBlockStfOutput};
use revm::{database::InMemoryDB, primitives::hardfork::SpecId};
use revm_primitives::alloy_primitives::B256;
use utils::generate_exec_update;
use zkaleido::ZkVmEnv;

#[derive(Clone)]
pub struct EvmConfig {
    pub chain_id: u64,
    pub spec_id: SpecId,
}

/// Executes the block with the given input and EVM configuration, returning public parameters.
pub fn process_block_transaction(mut input: EvmBlockStfInput) -> EvmBlockStfOutput {
    // Calculate the previous block hash
    let previous_block_hash = B256::from(keccak(alloy_rlp::encode(input.parent_header.clone())));

    // Deposit requests are processed and forwarded as public parameters for verification on the CL
    let deposit_requests = input.withdrawals.clone();

    // Initialize the in-memory database
    let _db = match InMemoryDB::initialize(&mut input) {
        Ok(database) => database,
        Err(e) => panic!("Failed to initialize database: {e:?}"),
    };

    // Construct the public parameters for the proof
    EvmBlockStfOutput {
        block_idx: Default::default(),
        new_blockhash: Default::default(),
        new_state_root: Default::default(),
        prev_blockhash: previous_block_hash,
        txn_root: Default::default(),
        deposit_requests,
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
        let input: EvmBlockStfInput = zkvm.read_serde();
        let output = process_block_transaction(input);

        if let Some(expected_hash) = current_blockhash {
            if output.prev_blockhash != expected_hash {
                println!(
                    "Block hash mismatch: expected {:?}, got {:?}",
                    expected_hash, output.prev_blockhash
                );
                return;
            }
        }

        current_blockhash = Some(output.new_blockhash);
        exec_updates.push(generate_exec_update(&output));
    }

    zkvm.commit_borsh(&exec_updates);
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

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
