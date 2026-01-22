use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use ssz::Encode;
use strata_checkpoint_types_ssz::CheckpointClaim;
use strata_ol_chain_types_new::OLBlock;
use strata_ol_state_types::OLState;
use zkaleido::{PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::statements::process_ol_stf;

#[derive(Debug)]
pub struct CheckpointProverInput {
    pub start_state: OLState,
    pub blocks: Vec<OLBlock>,
}

#[derive(Debug)]
pub struct CheckpointProgram;

impl ZkVmProgram for CheckpointProgram {
    type Input = CheckpointProverInput;
    type Output = CheckpointClaim;

    fn name() -> String {
        "OL STF".to_string()
    }

    fn proof_type() -> zkaleido::ProofType {
        zkaleido::ProofType::Groth16
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut input_builder = B::new();
        input_builder.write_buf(&input.start_state.as_ssz_bytes())?;
        input_builder.write_buf(&input.blocks.as_ssz_bytes())?;
        input_builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        H::extract_borsh_public_output(public_values)
    }
}

impl CheckpointProgram {
    pub fn native_host() -> NativeHost {
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_ol_stf(zkvm);
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
