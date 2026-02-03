use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

use rkyv::rancor::Error as RkyvError;
use zkaleido::{
    DataFormatError, ProofType, PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

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
        input_builder.write_serde(&el_inputs.len())?;

        for el_block_input in el_inputs {
            input_builder.write_serde(el_block_input)?;
        }

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

impl EvmEeProgram {
    pub fn native_host() -> NativeHost {
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_block_transaction_outer(zkvm);
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
