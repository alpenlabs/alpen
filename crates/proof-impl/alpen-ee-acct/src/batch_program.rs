//! ZkVmProgram implementation for batch proof (outer proof).

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

use ssz::Decode;
use strata_snark_acct_types::UpdateProofPubParams;
use zkaleido::{
    AggregationInput, ProofReceiptWithMetadata, ProofType, PublicValues, VerifyingKey, ZkVmError,
    ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::outer::process_batch_proof;

/// Output from batch proof (outer proof).
pub type BatchProofProgramOutput = UpdateProofPubParams;

/// Input for batch proof verification using zkaleido native types.
///
/// The chunk verification key is provided by the host and verified against
/// the hardcoded key in the guest for security.
#[derive(Debug, Clone)]
pub struct BatchProofInput {
    /// Chunk proof receipts with metadata (each contains Proof + PublicValues + Metadata)
    pub chunk_receipts: Vec<ProofReceiptWithMetadata>,

    /// Verification key for all chunk proofs (single key, same program)
    pub chunk_vkey: VerifyingKey,
}

impl BatchProofInput {
    pub fn new(chunk_receipts: Vec<ProofReceiptWithMetadata>, chunk_vkey: VerifyingKey) -> Self {
        Self {
            chunk_receipts,
            chunk_vkey,
        }
    }
}

/// The proof program for batch verification (outer proof).
#[derive(Debug)]
pub struct AlpenBatchProofProgram;

impl ZkVmProgram for AlpenBatchProofProgram {
    type Input = BatchProofInput;
    type Output = BatchProofProgramOutput;

    fn name() -> String {
        "Alpen EVM EE Batch Proof".to_string()
    }

    fn proof_type() -> ProofType {
        ProofType::Compressed
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut input_builder = B::new();

        // Write count as u32 (guest reads with read_serde)
        input_builder.write_serde(&(input.chunk_receipts.len() as u32))?;

        // Write each proof with vkey (guest reads with read_verified_buf)
        for receipt in &input.chunk_receipts {
            let agg_input = AggregationInput::new(receipt.clone(), input.chunk_vkey.clone());
            input_builder.write_proof(&agg_input)?;
        }

        input_builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        // The guest commits UpdateProofPubParams using SSZ serialization
        let output_bytes = public_values.as_bytes();
        let proof_output = UpdateProofPubParams::from_ssz_bytes(output_bytes)
            .map_err(|e| ZkVmError::Other(format!("Failed to decode SSZ output: {:?}", e)))?;
        Ok(proof_output)
    }
}

impl AlpenBatchProofProgram {
    /// Create a native host for testing without SP1
    pub fn native_host() -> NativeHost {
        const MOCK_CHUNK_VK: [u32; 8] = [0u32; 8];
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_batch_proof(zkvm, &MOCK_CHUNK_VK);
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
