//! ZkVmProgram implementation for ETH-EE account proof generation.
//!
//! This module defines the proof program structure that integrates with zkaleido's
//! proof generation framework.

use rsp_primitives::genesis::Genesis;
use ssz::Encode;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use zkaleido::{ProofType, PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::{AlpenEeProofOutput, CommitBlockPackage, process_alpen_ee_proof_update}; // From lib.rs

/// Private inputs to the proof for an Alpen EVM EE (runtime) account update.
///
/// This struct contains all the data needed by the guest zkVM to verify and apply
/// an EE account update operation for Alpen's EVM execution environment.
#[derive(Debug)]
pub struct AlpenEeProofInput {
    /// EeAccountState encoded as SSZ bytes
    astate_ssz: Vec<u8>,

    /// UpdateOperationData encoded as SSZ bytes
    operation_ssz: Vec<u8>,

    /// Previous ProofState (before the update) encoded as SSZ bytes
    /// The guest will verify that tree_hash_root(astate) matches this state
    prev_proof_state_ssz: Vec<u8>,

    /// Coinput witness data for messages
    coinputs: Vec<Vec<u8>>,

    /// Serialized block packages for building CommitChainSegment in guest.
    /// Each package contains execution metadata and raw block body.
    blocks: Vec<CommitBlockPackage>,

    /// Previous header (raw bytes)
    raw_prev_header: Vec<u8>,

    /// Partial pre-state (raw bytes)
    raw_partial_pre_state: Vec<u8>,

    /// Genesis data for constructing ChainSpec (serde-serialized)
    genesis: Genesis,
}

impl AlpenEeProofInput {
    /// Create a new AlpenEeProofInput
    pub fn new(
        astate_ssz: Vec<u8>,
        operation_ssz: Vec<u8>,
        prev_proof_state_ssz: Vec<u8>,
        coinputs: Vec<Vec<u8>>,
        blocks: Vec<CommitBlockPackage>,
        raw_prev_header: Vec<u8>,
        raw_partial_pre_state: Vec<u8>,
        genesis: Genesis,
    ) -> Self {
        Self {
            astate_ssz,
            operation_ssz,
            prev_proof_state_ssz,
            coinputs,
            blocks,
            raw_prev_header,
            raw_partial_pre_state,
            genesis,
        }
    }
}

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

        // Write SSZ-encoded data as raw buffers
        input_builder.write_buf(&input.astate_ssz)?;
        input_builder.write_buf(&input.operation_ssz)?;
        input_builder.write_buf(&input.prev_proof_state_ssz)?;

        // Write coinputs (Vec<Vec<u8>>) with SSZ
        input_builder.write_buf(&input.coinputs.as_ssz_bytes())?;

        // Write number of blocks with SSZ, then each block's data
        // Each block is a CommitBlockPackage: [exec_block_package (SSZ)][raw_block_body (strata_codec)]
        let num_blocks = input.blocks.len() as u32;
        input_builder.write_buf(&num_blocks.as_ssz_bytes())?;
        for block in &input.blocks {
            input_builder.write_buf(block.as_bytes())?;
        }

        // Write raw byte buffers
        input_builder.write_buf(&input.raw_prev_header)?;
        input_builder.write_buf(&input.raw_partial_pre_state)?;

        // Write genesis data (serde-serialized)
        input_builder.write_serde(&input.genesis)?;

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
