use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

use rkyv::rancor::Error as RkyvError;
use strata_ol_chain_types::{L2Block, L2BlockHeader};
use strata_ol_chainstate_types::Chainstate;
use strata_params::RollupParams;
use strata_primitives::buf::Buf32;
use zkaleido::{
    AggregationInput, DataFormatError, ProofReceiptWithMetadata, PublicValues, VerifyingKey,
    ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::process_cl_stf;

#[derive(Debug)]
pub struct ClStfInput {
    pub rollup_params: RollupParams,
    pub chainstate: Chainstate,
    pub parent_header: L2BlockHeader,
    pub l2_blocks: Vec<L2Block>,
    pub evm_ee_proof_with_vk: (ProofReceiptWithMetadata, VerifyingKey),
}

#[derive(Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
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
        let parent_header_bytes =
            rkyv::to_bytes::<RkyvError>(&input.parent_header).map_err(|err| {
                zkaleido::ZkVmInputError::DataFormat(DataFormatError::Other(err.to_string()))
            })?;
        input_builder.write_buf(parent_header_bytes.as_ref())?;

        let chainstate_bytes = rkyv::to_bytes::<RkyvError>(&input.chainstate).map_err(|err| {
            zkaleido::ZkVmInputError::DataFormat(DataFormatError::Other(err.to_string()))
        })?;
        input_builder.write_buf(chainstate_bytes.as_ref())?;

        let l2_blocks_bytes = rkyv::to_bytes::<RkyvError>(&input.l2_blocks).map_err(|err| {
            zkaleido::ZkVmInputError::DataFormat(DataFormatError::Other(err.to_string()))
        })?;
        input_builder.write_buf(l2_blocks_bytes.as_ref())?;

        let (proof, vk) = input.evm_ee_proof_with_vk.clone();
        input_builder.write_proof(&AggregationInput::new(proof, vk))?;

        input_builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        rkyv::from_bytes::<Self::Output, RkyvError>(public_values.as_bytes()).map_err(
            |err: RkyvError| ZkVmError::OutputExtractionError {
                source: DataFormatError::Other(err.to_string()),
            },
        )
    }
}

impl ClStfProgram {
    pub fn native_host() -> NativeHost {
        const MOCK_VK: [u32; 8] = [0u32; 8];
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_cl_stf(zkvm, &MOCK_VK);
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
