use k256::schnorr::SigningKey;
use rkyv::rancor::Error as RkyvError;
use rsp_primitives::genesis::Genesis;
use ssz::{Decode, Encode};
use strata_bridge_params::BridgeParams;
use strata_ee_chain_types::ChunkTransition;
use strata_ee_chunk_runtime::PrivateInput;
use strata_predicate::{PredicateKey, PredicateTypeId};
use zkaleido::{
    ProofType, PublicValues, ZkVmError, ZkVmInputError, ZkVmInputResult, ZkVmProgram, ZkVmResult,
};
use zkaleido_native_adapter::NativeHost;

use crate::process_ee_chunk;

fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[0x03u8; 32]).expect("valid test signing key")
}

/// Host-side input for the EE chunk proof.
#[derive(Debug)]
pub struct EeChunkProofInput {
    pub genesis: Genesis,
    pub private_input: PrivateInput,
    pub bridge_params: BridgeParams,
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
        ProofType::Groth16
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
        builder.write_buf(&input.bridge_params.as_ssz_bytes())?;
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
        NativeHost::new(test_signing_key(), process_ee_chunk)
    }

    /// Predicate key matching the signing key the native host uses, for wiring into
    /// functional-test params so the resulting witness verifies under `Bip340Schnorr`.
    pub fn test_predicate_key() -> PredicateKey {
        let pk = test_signing_key().verifying_key().to_bytes().to_vec();
        PredicateKey::new(PredicateTypeId::Bip340Schnorr, pk)
    }

    /// Executes the chunk proof program using the native host for testing.
    pub fn execute(
        input: &<Self as ZkVmProgram>::Input,
    ) -> ZkVmResult<<Self as ZkVmProgram>::Output> {
        let host = Self::native_host();
        let summary = <Self as ZkVmProgram>::execute(input, &host)?;
        <Self as ZkVmProgram>::process_output::<NativeHost>(summary.public_values())
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, sync::Arc};

    use alpen_reth_evm::evm::AlpenEvmFactory;
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
            .join("../../test-utils/data/evm_ee/witness_params.json");
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
            // This RSP fixture stores bytecodes as a Vec without original code-hash
            // keys. Re-hashing keeps the fixture behavior; production range
            // witnesses preserve the AccessedStateGenerator keys instead.
            witness
                .bytecodes
                .into_iter()
                .map(|bytecode| (bytecode.hash_slow(), bytecode))
                .collect(),
            witness.ancestor_headers,
        );

        // Build the EVM block from the witness.
        let header = witness.current_block.header().clone();
        let evm_header = EvmHeader::new(header.clone());
        let body = EvmBlockBody::from_alloy_body(witness.current_block.body().clone());
        let block = EvmBlock::new(evm_header, body);
        let tip_blkid: Hash = block.get_header().compute_block_id();
        let tip_state_root = block.get_header().get_state_root();
        let tip_exec_header_summary = block.get_header().get_exec_header_summary();

        // Execute the block to get outputs.
        let chain_spec: Arc<reth_chainspec::ChainSpec> =
            Arc::new((&witness.genesis).try_into().unwrap());
        let ee = EvmExecutionEnvironment::new(chain_spec, AlpenEvmFactory::default());
        let exec_payload = ExecPayload::new(&header, block.get_body());
        let inputs = ExecInputs::new_empty();
        let output = ee
            .execute_block_body(&pre_state, &exec_payload, &inputs)
            .expect("block execution should succeed");
        let outputs = output.outputs().clone();

        // Build chunk transition.
        let chunk_transition = ChunkTransition::new(
            parent_blkid,
            tip_blkid,
            tip_state_root,
            tip_exec_header_summary,
            inputs.clone(),
            outputs.clone(),
        );

        // Encode block, header, and the block's per-block witness for the
        // private input. With a single block in this chunk, the block's witness
        // is the pre-state anchored at the parent root.
        let raw_pre_state = encode_to_vec(&pre_state).expect("encode pre-state");
        let raw_block_data = RawBlockData::from_block::<EvmExecutionEnvironment>(
            &block,
            inputs,
            outputs,
            raw_pre_state,
        )
        .expect("encode block");
        let raw_chunk = RawChunkData::new(vec![raw_block_data], parent_blkid);
        let raw_prev_header = encode_to_vec(&parent_evm_header).expect("encode prev header");

        let private_input = PrivateInput::new(chunk_transition.clone(), raw_chunk, raw_prev_header);

        let proof_input = EeChunkProofInput {
            genesis: witness.genesis,
            private_input,
            bridge_params: BridgeParams::default(),
        };

        // Run the full native execution pipeline.
        let result =
            EeChunkProgram::execute(&proof_input).expect("native execution should succeed");

        assert_eq!(result.parent_exec_blkid(), parent_blkid);
        assert_eq!(result.tip_exec_blkid(), tip_blkid);
    }
}
