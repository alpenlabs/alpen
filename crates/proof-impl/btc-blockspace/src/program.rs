use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

use bitcoin::consensus::serialize;
use strata_primitives::l1::L1TxProof;
use zkaleido::{
    ProofType, PublicValues, ZkVmError, ZkVmHost, ZkVmInputBuilder, ZkVmInputResult, ZkVmProgram,
    ZkVmProgramPerf, ZkVmResult,
};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::{
    block::witness_commitment_from_coinbase,
    logic::{process_blockscan_proof, BlockScanProofInput, BlockscanProofOutput},
};

#[derive(Debug)]
pub struct BtcBlockspaceProgram;

impl ZkVmProgram for BtcBlockspaceProgram {
    type Input = BlockScanProofInput;
    type Output = BlockscanProofOutput;

    fn name() -> String {
        "Bitcoin Blockspace".to_string()
    }

    fn proof_type() -> ProofType {
        ProofType::Compressed
    }

    /// Prepares the input for the zkVM.
    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: ZkVmInputBuilder<'a>,
    {
        let mut zkvm_input = B::new();

        zkvm_input.write_serde(&input.btc_blocks.len())?;
        zkvm_input.write_borsh(&input.tx_filters)?;

        for block in &input.btc_blocks {
            let inclusion_proof = witness_commitment_from_coinbase(&block.txdata[0])
                .map(|_| L1TxProof::generate(&block.txdata, 0));

            let serialized_block = serialize(block);
            zkvm_input
                .write_buf(&serialized_block)?
                .write_borsh(&inclusion_proof)?;
        }

        zkvm_input.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: ZkVmHost,
    {
        H::extract_borsh_public_output(public_values)
    }
}

impl ZkVmProgramPerf for BtcBlockspaceProgram {}

impl BtcBlockspaceProgram {
    pub fn native_host() -> NativeHost {
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_blockscan_proof(zkvm);
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
