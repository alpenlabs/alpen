use zkaleido::{ProofType, PublicValues, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::NativeHost;

use crate::{
    primitives::{EvmEeProofInput, EvmEeProofOutput},
    process_block_transaction_outer,
};

#[derive(Debug)]
pub struct EvmEeProgram;

impl ZkVmProgram for EvmEeProgram {
    type Input = EvmEeProofInput;
    type Output = EvmEeProofOutput;

    fn name() -> String {
        "EVM EE STF".to_string()
    }

    fn proof_type() -> ProofType {
        ProofType::Compressed
    }

    fn prepare_input<'a, B>(el_inputs: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut input_builder = B::new();
        input_builder.write_serde(&el_inputs.block_inputs.len())?;
        input_builder.write_serde(&el_inputs.bridge_params)?;

        for el_block_input in &el_inputs.block_inputs {
            input_builder.write_serde(el_block_input)?;
        }

        input_builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        H::extract_borsh_public_output(public_values)
    }
}

impl EvmEeProgram {
    pub fn native_host() -> NativeHost {
        NativeHost::new_with_random_key(process_block_transaction_outer)
    }

    // Add this new convenience method
    pub fn execute(
        input: &<Self as ZkVmProgram>::Input,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        // Get the native host and delegate to the trait's execute method
        let host = Self::native_host();
        let summary = <Self as ZkVmProgram>::execute(input, &host)?;
        <Self as ZkVmProgram>::process_output::<NativeHost>(summary.public_values())
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::read_to_string, path::PathBuf};

    use rsp_client_executor::io::EthClientExecutorInput;
    use strata_bridge_params::BridgeParams;

    use super::*;
    use crate::primitives::EvmEeProofInput;

    fn get_mock_input() -> EthClientExecutorInput {
        let json_content = read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../test-utils/data/evm_ee/witness_params.json"),
        )
        .expect("Failed to read the blob data file");

        #[derive(serde::Deserialize)]
        struct TestData {
            witness: EthClientExecutorInput,
        }

        serde_json::from_str::<TestData>(&json_content)
            .expect("Valid json")
            .witness
    }

    #[test]
    fn public_output_commits_bridge_params() {
        let bridge_params =
            BridgeParams::new_with_descriptor_limit(100_000_000, Some(1_000_000_000), 100)
                .expect("valid bridge params");
        let input = EvmEeProofInput::new(bridge_params, vec![get_mock_input()]);

        let output = EvmEeProgram::execute(&input).expect("native execution succeeds");

        assert_eq!(
            output.bridge_params().denomination(),
            bridge_params.denomination()
        );
        assert_eq!(
            output.bridge_params().max_withdrawal_amount(),
            bridge_params.max_withdrawal_amount()
        );
        assert_eq!(
            output.bridge_params().max_withdrawal_descriptor_len(),
            bridge_params.max_withdrawal_descriptor_len()
        );
        assert_eq!(output.segments().len(), 1);
    }
}
