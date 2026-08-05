//! EVM Execution Environment STF for Alpen prover, using RSP for EVM execution. Provides primitives
//! and utilities to process Ethereum block transactions and state transitions in a zkVM.
pub mod executor;
pub mod primitives;

pub use primitives::{EvmBlockStfInput, EvmBlockStfOutput};

#[cfg(test)]
mod tests {

    use std::{fs::read_to_string, path::PathBuf};

    use serde::{Deserialize, Serialize};
    use strata_bridge_params::BridgeParams;

    use super::{executor::process_block, EvmBlockStfInput, EvmBlockStfOutput};

    #[derive(Serialize, Deserialize)]
    struct TestData {
        witness: EvmBlockStfInput,
        params: EvmBlockStfOutput,
    }

    fn get_mock_data() -> TestData {
        let json_content = read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../test-utils/data/evm_ee/witness_params.json"),
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
        let op = process_block(input, BridgeParams::default())
            .expect("Failed to process block transaction");
        assert_eq!(op, test_data.params);
    }
}
