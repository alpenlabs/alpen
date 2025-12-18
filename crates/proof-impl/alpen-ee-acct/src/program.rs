//! ZkVmProgram implementation for ETH-EE account proof generation.
//!
//! This module defines the proof program structure that integrates with zkaleido's
//! proof generation framework.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use strata_codec::encode_to_vec;
use zkaleido::{
    ProofType, PublicValues, ZkVmError, ZkVmInputError, ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::{AlpenEeProofOutput, process_alpen_ee_proof_update, types::AlpenEeProofInput};

/// Output from Alpen EE account proof.
/// This is the public output committed by the guest.
pub type AlpenEeProofProgramOutput = AlpenEeProofOutput;

/// The proof program for Alpen EVM EE account updates.
#[derive(Debug)]
pub struct AlpenEeProofProgram;

impl ZkVmProgram for AlpenEeProofProgram {
    type Input = AlpenEeProofInput;
    type Output = AlpenEeProofProgramOutput;

    fn name() -> String {
        "Alpen EVM EE Account STF".to_string()
    }

    fn proof_type() -> ProofType {
        ProofType::Compressed
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut input_builder = B::new();

        // Serialize and write account initialization data as a single Codec blob
        let account_init_bytes = encode_to_vec(input.account_init()).map_err(|e| {
            ZkVmInputError::InputBuild(format!("Failed to encode account_init: {}", e))
        })?;
        input_builder.write_buf(&account_init_bytes)?;

        // Serialize and write runtime update input as a single Codec blob
        let runtime_input_bytes = encode_to_vec(input.runtime_input()).map_err(|e| {
            ZkVmInputError::InputBuild(format!("Failed to encode runtime_input: {}", e))
        })?;
        input_builder.write_buf(&runtime_input_bytes)?;

        // Write genesis data (serde-serialized)
        input_builder.write_serde(input.genesis())?;

        input_builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        // The guest commits the full AlpenEeProofOutput using borsh
        let proof_output: AlpenEeProofOutput = H::extract_borsh_public_output(public_values)?;
        Ok(proof_output)
    }
}

impl AlpenEeProofProgram {
    /// Create a native host for testing without SP1
    pub fn native_host() -> NativeHost {
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_alpen_ee_proof_update(zkvm);
                }))
                .map_err(|_| ZkVmError::ExecutionError(Self::name()))?;
                Ok(())
            })),
        }
    }

    /// Execute the program with native host (for testing)
    pub fn execute(
        input: &<Self as ZkVmProgram>::Input,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        let host = Self::native_host();
        <Self as ZkVmProgram>::execute(input, &host)
    }
}
