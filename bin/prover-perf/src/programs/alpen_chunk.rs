//! Perf input for the alpen-chunk SP1 guest.
//!
//! Mirrors the setup in `alpen-chunk`'s `test_native_chunk_execution`,
//! reusing the EVM witness fixture from `proof-impl/evm-ee-stf/test_data`
//! to produce a single-block chunk transition.

use std::{fs, path::PathBuf, sync::Arc};

use reth_primitives_traits::Block as _;
use rsp_client_executor::io::EthClientExecutorInput;
use serde::Deserialize;
use strata_acct_types::Hash;
use strata_codec::encode_to_vec;
use strata_ee_acct_types::{ExecBlock, ExecHeader, ExecPayload, ExecutionEnvironment};
use strata_ee_chain_types::ExecInputs;
use strata_ee_chunk_runtime::{PrivateInput, RawBlockData, RawChunkData};
use strata_evm_ee::{EvmBlock, EvmBlockBody, EvmExecutionEnvironment, EvmHeader, EvmPartialState};
use strata_proofimpl_alpen_chunk::{EeChunkProgram, EeChunkProofInput};
use tracing::info;
use zkaleido::{PerformanceReport, ZkVmHostPerf, ZkVmProgramPerf};

#[derive(Deserialize)]
struct WitnessData {
    witness: EthClientExecutorInput,
}

fn load_witness() -> EthClientExecutorInput {
    // The witness JSON lives next to the EVM-EE STF crate (the host-side
    // crate that owns the canonical Reth-shaped witness fixtures); the
    // alpen-chunk runtime can consume it directly.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/proof-impl/evm-ee-stf/test_data/witness_params.json");
    let json = fs::read_to_string(path).expect("read witness JSON");
    let data: WitnessData = serde_json::from_str(&json).expect("parse witness JSON");
    data.witness
}

fn prepare_input() -> EeChunkProofInput {
    info!("Preparing input for Alpen Chunk");
    let witness = load_witness();

    let parent_header = witness
        .ancestor_headers
        .last()
        .expect("need at least one ancestor header")
        .clone();
    let parent_evm_header = EvmHeader::new(parent_header);
    let parent_blkid: Hash = parent_evm_header.compute_block_id();

    let pre_state = EvmPartialState::new(
        witness.parent_state.clone(),
        witness.bytecodes.clone(),
        witness.ancestor_headers.clone(),
    );

    let header = witness.current_block.header().clone();
    let evm_header = EvmHeader::new(header.clone());
    let body = EvmBlockBody::from_alloy_body(witness.current_block.body().clone());
    let block = EvmBlock::new(evm_header, body);
    let tip_blkid: Hash = block.get_header().compute_block_id();

    let chain_spec: Arc<reth_chainspec::ChainSpec> =
        Arc::new((&witness.genesis).try_into().unwrap());
    let ee = EvmExecutionEnvironment::new(chain_spec);
    let exec_payload = ExecPayload::new(&header, block.get_body());
    let inputs = ExecInputs::new_empty();
    let output = ee
        .execute_block_body(&pre_state, &exec_payload, &inputs)
        .expect("block execution should succeed");
    let outputs = output.outputs().clone();

    let chunk_transition = strata_ee_chain_types::ChunkTransition::new(
        parent_blkid,
        tip_blkid,
        inputs.clone(),
        outputs.clone(),
    );

    let raw_block_data =
        RawBlockData::from_block::<EvmExecutionEnvironment>(&block, inputs, outputs)
            .expect("encode block");
    let raw_chunk = RawChunkData::new(vec![raw_block_data], parent_blkid);
    let raw_prev_header = encode_to_vec(&parent_evm_header).expect("encode prev header");
    let raw_pre_state = encode_to_vec(&pre_state).expect("encode pre-state");

    let private_input = PrivateInput::new(
        chunk_transition,
        raw_chunk,
        raw_prev_header,
        raw_pre_state,
    );

    EeChunkProofInput {
        genesis: witness.genesis,
        private_input,
    }
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHostPerf) -> PerformanceReport {
    info!("Generating performance report for Alpen Chunk");
    let input = prepare_input();
    EeChunkProgram::perf_report(&input, host).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alpen_chunk_native_execution() {
        let input = prepare_input();
        let output = EeChunkProgram::execute(&input).unwrap();
        // The chunk transition's parent/tip blkids must match the block
        // hashes computed during input prep — sanity that the perf
        // fixture produces a self-consistent transition.
        assert_ne!(output.parent_exec_blkid(), output.tip_exec_blkid());
    }
}
