//! EVM Execution Environment STF for Alpen prover, using RSP for EVM execution. Provides primitives
//! and utilities to process Ethereum block transactions and state transitions in a zkVM.
pub mod executor;
pub mod primitives;
pub mod program;
pub mod utils;

pub use primitives::{EvmBlockStfInput, EvmBlockStfOutput};
use rsp_client_executor::io::EthClientExecutorInput;
use utils::generate_exec_update;
use zkaleido::ZkVmEnv;

use crate::executor::process_block;

/// Processes a sequence of EL block transactions from the given `zkvm` environment, ensuring block
/// hash continuity and committing the resulting updates.
pub fn process_block_transaction_outer(zkvm: &impl ZkVmEnv) {
    let num_blocks: u32 = zkvm.read_serde();
    assert!(num_blocks > 0, "At least one block is required.");

    let mut exec_updates = Vec::with_capacity(num_blocks as usize);
    let mut current_blockhash = None;

    for _ in 0..num_blocks {
        let input: EthClientExecutorInput = zkvm.read_serde();
        let output = process_block(input).expect("Failed to process block transaction");

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

    use super::{process_block, EvmBlockStfInput, EvmBlockStfOutput};

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
        let op = process_block(input).expect("Failed to process block transaction");
        assert_eq!(op, test_data.params);
    }
}
