use rkyv::rancor::Error as RkyvError;
use rsp_primitives::genesis::Genesis;
use ssz::Decode;
use strata_ee_chain_types::ChunkTransition;
use strata_ee_chunk_runtime::PrivateInput;
use zkaleido::{
    ProofType, PublicValues, ZkVmError, ZkVmInputError, ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::NativeHost;

use crate::process_ee_chunk;

/// Host-side input for the EE chunk proof.
#[derive(Debug)]
pub struct EeChunkProofInput {
    pub genesis: Genesis,
    pub private_input: PrivateInput,
}

#[derive(Debug)]
pub struct EeChunkProgram;

impl ZkVmProgram for EeChunkProgram {
    type Input = EeChunkProofInput;
    type Output = ChunkTransition;

    fn name() -> String {
        "EVM EE Chunk".to_string()
    }

    fn proof_type() -> ProofType {
        ProofType::Compressed
    }

    fn prepare_input<'a, B>(input: &'a Self::Input) -> ZkVmInputResult<B::Input>
    where
        B: zkaleido::ZkVmInputBuilder<'a>,
    {
        let mut builder = B::new();
        builder.write_serde(&input.genesis)?;
        let rkyv_bytes = rkyv::to_bytes::<RkyvError>(&input.private_input)
            .map_err(|e| ZkVmInputError::InputBuild(e.to_string()))?;
        builder.write_buf(&rkyv_bytes)?;
        builder.build()
    }

    fn process_output<H>(public_values: &PublicValues) -> ZkVmResult<Self::Output>
    where
        H: zkaleido::ZkVmHost,
    {
        ChunkTransition::from_ssz_bytes(public_values.as_bytes())
            .map_err(|e| ZkVmError::Other(e.to_string()))
    }
}

impl EeChunkProgram {
    pub fn native_host() -> NativeHost {
        NativeHost::new(process_ee_chunk)
    }

    /// Executes the chunk proof program using the native host for testing.
    pub fn execute(
        input: &<Self as ZkVmProgram>::Input,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        let host = Self::native_host();
        <Self as ZkVmProgram>::execute(input, &host)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, sync::Arc};

    use reth_primitives_traits::Block as _;
    use rsp_client_executor::io::EthClientExecutorInput;
    use serde::Deserialize;
    use strata_acct_types::Hash;
    use strata_codec::encode_to_vec;
    use strata_ee_acct_types::{ExecBlock, ExecHeader, ExecPayload, ExecutionEnvironment};
    use strata_ee_chain_types::{ChunkTransition, ExecInputs};
    use strata_ee_chunk_runtime::{PrivateInput, RawBlockData, RawChunkData};
    use strata_evm_ee::{
        EvmBlock, EvmBlockBody, EvmExecutionEnvironment, EvmHeader, EvmPartialState,
    };

    use super::*;

    #[derive(Deserialize)]
    struct WitnessData {
        witness: EthClientExecutorInput,
    }

    fn load_witness() -> EthClientExecutorInput {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../proof-impl/evm-ee-stf/test_data/witness_params.json");
        let json = fs::read_to_string(path).expect("read witness JSON");
        let data: WitnessData = serde_json::from_str(&json).expect("parse witness JSON");
        data.witness
    }

    #[test]
    fn test_native_chunk_execution() {
        let witness = load_witness();

        // Extract parent header (last ancestor = direct parent of current block).
        let parent_header = witness
            .ancestor_headers
            .last()
            .expect("need at least one ancestor header")
            .clone();
        let parent_evm_header = EvmHeader::new(parent_header);
        let parent_blkid: Hash = parent_evm_header.compute_block_id();

        // Build partial pre-state from witness data.
        let pre_state = EvmPartialState::new(
            witness.parent_state,
            witness.bytecodes,
            witness.ancestor_headers,
        );

        // Build the EVM block from the witness.
        let header = witness.current_block.header().clone();
        let evm_header = EvmHeader::new(header.clone());
        let body = EvmBlockBody::from_alloy_body(witness.current_block.body().clone());
        let block = EvmBlock::new(evm_header, body);
        let tip_blkid: Hash = block.get_header().compute_block_id();

        // Execute the block to get outputs.
        let chain_spec: Arc<reth_chainspec::ChainSpec> =
            Arc::new((&witness.genesis).try_into().unwrap());
        let ee = EvmExecutionEnvironment::new(chain_spec);
        let exec_payload = ExecPayload::new(&header, block.get_body());
        let inputs = ExecInputs::new_empty();
        let output = ee
            .execute_block_body(&pre_state, &exec_payload, &inputs)
            .expect("block execution should succeed");
        let outputs = output.outputs().clone();

        // Build chunk transition.
        let chunk_transition =
            ChunkTransition::new(parent_blkid, tip_blkid, inputs.clone(), outputs.clone());

        // Encode block, header, and state for the private input.
        let raw_block_data =
            RawBlockData::from_block::<EvmExecutionEnvironment>(&block, inputs, outputs)
                .expect("encode block");
        let raw_chunk = RawChunkData::new(vec![raw_block_data], parent_blkid);
        let raw_prev_header = encode_to_vec(&parent_evm_header).expect("encode prev header");
        let raw_pre_state = encode_to_vec(&pre_state).expect("encode pre-state");

        let private_input = PrivateInput::new(
            chunk_transition.clone(),
            raw_chunk,
            raw_prev_header,
            raw_pre_state,
        );

        let proof_input = EeChunkProofInput {
            genesis: witness.genesis,
            private_input,
        };

        // Run the full native execution pipeline.
        let result =
            EeChunkProgram::execute(&proof_input).expect("native execution should succeed");

        assert_eq!(result.parent_exec_blkid(), parent_blkid);
        assert_eq!(result.tip_exec_blkid(), tip_blkid);
    }
}
