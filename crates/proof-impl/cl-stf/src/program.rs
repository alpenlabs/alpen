use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::{buf::Buf32, params::RollupParams};
use strata_state::{batch::TxFilterConfigTransition, block::L2Block, chain_state::Chainstate};
use zkaleido::{
    AggregationInput, ProofReceipt, PublicValues, VerifyingKey, ZkVmError, ZkVmInputResult,
    ZkVmProgram, ZkVmProgramPerf, ZkVmResult,
};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::process_cl_stf;

#[derive(Debug)]
pub struct ClStfInput {
    pub rollup_params: RollupParams,
    pub chainstate: Chainstate,
    pub l2_blocks: Vec<L2Block>,
    pub evm_ee_proof_with_vk: (ProofReceipt, VerifyingKey),
    pub btc_blockspace_proof_with_vk: Option<(ProofReceipt, VerifyingKey)>,
}

#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct ClStfOutput {
    pub epoch: u64,
    pub initial_chainstate_root: Buf32,
    pub final_chainstate_root: Buf32,
    pub tx_filters_transition: Option<TxFilterConfigTransition>,
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
        input_builder.write_borsh(&input.chainstate)?;
        input_builder.write_borsh(&input.l2_blocks)?;

        match input.btc_blockspace_proof_with_vk.clone() {
            Some((proof, vk)) => {
                input_builder.write_serde(&true)?;
                input_builder.write_proof(&AggregationInput::new(proof, vk))?;
            }
            None => {
                input_builder.write_serde(&false)?;
            }
        };

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

impl ZkVmProgramPerf for ClStfProgram {}

impl ClStfProgram {
    pub fn native_host() -> NativeHost {
        const MOCK_VK: [u32; 8] = [0u32; 8];
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_cl_stf(zkvm, &MOCK_VK, &MOCK_VK);
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
