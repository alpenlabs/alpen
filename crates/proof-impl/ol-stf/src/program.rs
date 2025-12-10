use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

use borsh::{BorshDeserialize, BorshSerialize};
use strata_ol_chain_types::{L2Block, L2BlockHeader};
use strata_ol_chainstate_types::Chainstate;
use strata_params::RollupParams;
use strata_primitives::buf::Buf32;
use zkaleido::{
    AggregationInput, ProofReceiptWithMetadata, PublicValues, VerifyingKey, ZkVmError,
    ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::process_ol_stf;

#[derive(Debug)]
pub struct OLStfInput {
    pub rollup_params: RollupParams,
    pub chainstate: Chainstate,
    pub parent_header: L2BlockHeader,
    pub l2_blocks: Vec<L2Block>,
    pub evm_ee_proof_with_vk: (ProofReceiptWithMetadata, VerifyingKey),
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct OLStfOutput {
    pub epoch: u64,
    pub initial_chainstate_root: Buf32,
    pub final_chainstate_root: Buf32,
}

#[derive(Debug)]
pub struct OLStfProgram;

impl ZkVmProgram for OLStfProgram {
    type Input = OLStfInput;
    type Output = OLStfOutput;

    fn name() -> String {
        "OL STF".to_string()
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

impl OLStfProgram {
    pub fn native_host() -> NativeHost {
        const MOCK_VK: [u32; 8] = [0u32; 8];
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_ol_stf(zkvm, &MOCK_VK);
                }))
                .map_err(|_| ZkVmError::ExecutionError(Self::name()))?;
                Ok(())
            })),
        }
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
