use borsh::{BorshDeserialize, BorshSerialize};
use strata_ol_chain_types::{L2Block, L2BlockHeader};
use strata_ol_chainstate_types::Chainstate;
use strata_params::RollupParams;
use strata_primitives::buf::Buf32;
use zkaleido::{
    AggregationInput, ProofReceiptWithMetadata, PublicValues, VerifyingKey, ZkVmInputResult,
    ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::NativeHost;

use crate::process_cl_stf;

#[derive(Debug)]
pub struct ClStfInput {
    pub rollup_params: RollupParams,
    pub chainstate: Chainstate,
    pub parent_header: L2BlockHeader,
    pub l2_blocks: Vec<L2Block>,
    pub evm_ee_proof_with_vk: (ProofReceiptWithMetadata, VerifyingKey),
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct ClStfOutput {
    pub epoch: u64,
    pub initial_chainstate_root: Buf32,
    pub final_chainstate_root: Buf32,
}

#[derive(Debug)]
pub struct ClStfProgram;

impl ZkVmProgram for ClStfProgram {
    type Input = ClStfInput;
    type Output = ClStfOutput;

    fn name() -> String {
        "CL STF".to_string()
    }

    fn proof_type() -> zkaleido::ProofType {
        zkaleido::ProofType::Compressed
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut input_builder = B::new();
        input_builder.write_serde(&input.rollup_params)?;
        input_builder.write_borsh(&input.parent_header)?;
        input_builder.write_borsh(&input.chainstate)?;
        input_builder.write_borsh(&input.l2_blocks)?;

        let (proof, vk) = input.evm_ee_proof_with_vk.clone();
        input_builder.write_proof(&AggregationInput::new(proof, vk))?;

        input_builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        H::extract_borsh_public_output(public_values)
    }
}

impl ClStfProgram {
    pub fn native_host() -> NativeHost {
        const MOCK_VK: [u32; 8] = [0u32; 8];
        NativeHost::new(move |zkvm| {
            process_cl_stf(zkvm, &MOCK_VK);
        })
    }

    // Add this new convenience method
    pub fn execute(
        input: &<Self as ZkVmProgram>::Input,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        // Get the native host and delegate to the trait's execute method
        let host = Self::native_host();
        <Self as ZkVmProgram>::execute(input, &host)
    }
}
