//! ZkVmProgram implementation for ETH-EE account proof generation.
//!
//! This module defines the proof program structure that integrates with zkaleido's
//! proof generation framework.

use std::{
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

use zkaleido::{ProofType, PublicValues, ZkVmError, ZkVmInputResult, ZkVmProgram, ZkVmResult};
use zkaleido_native_adapter::{NativeHost, NativeMachine};

use crate::process_eth_ee_acct_update;  // From lib.rs

/// Input for ETH-EE account proof generation
/// This is the high-level input that the host provides
#[derive(Debug)]
pub struct EthEeAcctInput {
    /// EeAccountState encoded as SSZ bytes
    pub astate_ssz: Vec<u8>,

    /// UpdateOperationData encoded as SSZ bytes
    pub operation_ssz: Vec<u8>,

    /// Coinput witness data for messages
    pub coinputs: Vec<Vec<u8>>,

    /// Each CommitChainSegment encoded as SSZ bytes
    pub commit_segments_ssz: Vec<Vec<u8>>,

    /// Previous header (raw bytes)
    pub raw_prev_header: Vec<u8>,

    /// Partial pre-state (raw bytes)
    pub raw_partial_pre_state: Vec<u8>,

    /// Genesis data for constructing ChainSpec (serde-serialized)
    pub genesis: rsp_primitives::genesis::Genesis,
}

/// Output from ETH-EE account proof
/// This is the public output committed by the guest
#[derive(Clone, Debug)]
pub struct EthEeAcctOutput {
    /// Hash of the new EE account state after update
    pub new_state_hash: [u8; 32],
}

/// The proof program for ETH-EE account updates
#[derive(Debug)]
pub struct EthEeAcctProgram;

impl ZkVmProgram for EthEeAcctProgram {
    type Input = EthEeAcctInput;
    type Output = EthEeAcctOutput;

    fn name() -> String {
        "ETH-EE Account STF".to_string()
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

        // Write coinputs (Vec<Vec<u8>>) with borsh
        input_builder.write_borsh(&input.coinputs)?;

        // Write number of segments, then each segment buffer
        input_builder.write_borsh(&(input.commit_segments_ssz.len() as u32))?;
        for segment_ssz in &input.commit_segments_ssz {
            input_builder.write_buf(segment_ssz)?;
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
        // The guest commits the state hash as [u8; 32] using borsh
        let new_state_hash: [u8; 32] = H::extract_borsh_public_output(public_values)?;
        Ok(EthEeAcctOutput { new_state_hash })
    }
}

impl EthEeAcctProgram {
    /// Create a native host for testing without SP1
    pub fn native_host() -> NativeHost {
        NativeHost {
            process_proof: Arc::new(Box::new(move |zkvm: &NativeMachine| {
                catch_unwind(AssertUnwindSafe(|| {
                    process_eth_ee_acct_update(zkvm);
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
