//! ZkVmProgram implementation for chunk proof (inner proof).

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use rsp_primitives::genesis::Genesis;
use ssz::{Decode, Encode};
use strata_ee_acct_runtime::UpdateTransitionData;
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_snark_acct_types::ProofState;
use zkaleido::{
    ProofType, PublicValues, ZkVmError, ZkVmInputError, ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::{inner::process_chunk_proof, types::ChunkProofOutput};

/// Output from chunk proof (inner proof).
pub type ChunkProofProgramOutput = ChunkProofOutput;

/// Encoded full block (header + body) bytes.
type BlockBytes = Vec<u8>;

/// Coinput bytes for message verification.
type CoinputBytes = Vec<u8>;

/// Input data for chunk proof.
///
/// This is a data container for passing to prepare_input - it does NOT need to implement
/// serialization traits. We write each field as separate buffers to avoid extra layers.
#[derive(Debug, Clone)]
pub struct ChunkProofInput {
    pub astate: EeAccountState,
    pub prev_proof_state: ProofState,
    pub update_transition: UpdateTransitionData,
    pub coinputs: Vec<CoinputBytes>,
    pub exec_block_packages: Vec<ExecBlockPackage>,
    pub block_bytes: Vec<BlockBytes>,
    pub raw_prev_header: Vec<u8>,
    pub raw_partial_pre_state: Vec<u8>,
    pub genesis: Genesis,
}

/// The proof program for chunk execution (inner proof).
#[derive(Debug)]
pub struct AlpenChunkProofProgram;

impl ZkVmProgram for AlpenChunkProofProgram {
    type Input = ChunkProofInput;
    type Output = ChunkProofProgramOutput;

    fn name() -> String {
        "Alpen EVM EE Chunk Proof".to_string()
    }

    fn proof_type() -> ProofType {
        ProofType::Compressed
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut input_builder = B::new();

        // Validate that exec_block_packages and block_bytes have matching lengths
        if input.exec_block_packages.len() != input.block_bytes.len() {
            return Err(ZkVmInputError::InputBuild(format!(
                "Length mismatch: {} exec_block_packages vs {} block_bytes",
                input.exec_block_packages.len(),
                input.block_bytes.len()
            )));
        }

        // Write account state, proof state, and update transition
        input_builder.write_buf(&input.astate.as_ssz_bytes())?;
        input_builder.write_buf(&input.prev_proof_state.as_ssz_bytes())?;
        input_builder.write_buf(&input.update_transition.as_ssz_bytes())?;

        // Write coinputs: count + items
        let coinputs_count = input.coinputs.len() as u32;
        input_builder.write_buf(&coinputs_count.to_le_bytes())?;
        for coinput in &input.coinputs {
            input_builder.write_buf(coinput)?;
        }

        // Write exec_block_packages: count + items
        let packages_count = input.exec_block_packages.len() as u32;
        input_builder.write_buf(&packages_count.to_le_bytes())?;
        for package in &input.exec_block_packages {
            input_builder.write_buf(&package.as_ssz_bytes())?;
        }

        // Write block_bytes: count + items
        let blocks_count = input.block_bytes.len() as u32;
        input_builder.write_buf(&blocks_count.to_le_bytes())?;
        for block in &input.block_bytes {
            input_builder.write_buf(block)?;
        }

        // Write raw buffers
        input_builder.write_buf(&input.raw_prev_header)?;
        input_builder.write_buf(&input.raw_partial_pre_state)?;

        // Write genesis
        input_builder.write_serde(&input.genesis)?;

        input_builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        // The guest commits ChunkProofOutput using SSZ serialization
        let output_bytes = public_values.as_bytes();
        let chunk_output = ChunkProofOutput::from_ssz_bytes(output_bytes)
            .map_err(|e| ZkVmError::Other(format!("Failed to decode SSZ output: {:?}", e)))?;
        Ok(chunk_output)
    }
}

impl AlpenChunkProofProgram {
    /// Create a native host for testing without SP1
    pub fn native_host() -> NativeHost {
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_chunk_proof(zkvm);
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
