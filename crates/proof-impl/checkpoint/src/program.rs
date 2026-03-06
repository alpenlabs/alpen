use strata_checkpoint_types::BatchInfo;
use zkaleido::{PublicValues, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::NativeHost;

use crate::process_checkpoint_proof;

#[derive(Debug)]
pub struct CheckpointProgram;

impl ZkVmProgram for CheckpointProgram {
    type Input = BatchInfo;
    type Output = BatchInfo;

    fn name() -> String {
        "Checkpoint".to_string()
    }

    fn proof_type() -> zkaleido::ProofType {
        zkaleido::ProofType::Groth16
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        B::new().write_borsh(&input)?.build()
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
        NativeHost::new(move |zkvm| {
            process_checkpoint_proof(zkvm);
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
