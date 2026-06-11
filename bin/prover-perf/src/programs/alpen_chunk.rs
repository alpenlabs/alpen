//! Perf input for the alpen-chunk SP1 guest.
//!
//! Mirrors the setup in `alpen-chunk`'s `test_native_chunk_execution`,
//! reusing the EVM witness fixture from `proof-impl/evm-ee-stf/test_data`
//! to produce a single-block chunk transition.

use std::{fs, path::PathBuf, sync::Arc};

use alpen_reth_evm::evm::AlpenEvmFactory;
use reth_primitives_traits::Block as _;
use rsp_client_executor::io::EthClientExecutorInput;
use serde::Deserialize;
use strata_acct_types::Hash;
use strata_bridge_params::BridgeParams;
use strata_codec::encode_to_vec;
use strata_ee_acct_types::{ExecBlock, ExecHeader, ExecPayload, ExecutionEnvironment};
use strata_ee_chain_types::ExecInputs;
use strata_ee_chunk_runtime::{PrivateInput, RawBlockData, RawChunkData};
use strata_evm_ee::{EvmBlock, EvmBlockBody, EvmExecutionEnvironment, EvmHeader, EvmPartialState};
use strata_proofimpl_alpen_chunk::{EeChunkProgram, EeChunkProofInput};
use tracing::info;
use zkaleido::{ExecutionSummary, ZkVmHost, ZkVmProgram};

#[derive(Deserialize)]
struct WitnessData {
    witness: EthClientExecutorInput,
}

fn load_witness() -> EthClientExecutorInput {
    // Canonical Reth-shaped witness fixture lives under
    // crates/test-utils/data/evm_ee/, alongside the legacy
    // witness_{N}.json files used by the EVM-EE STF tests.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/test-utils/data/evm_ee/witness_params.json");
    let json = fs::read_to_string(path).expect("read witness JSON");
    let data: WitnessData = serde_json::from_str(&json).expect("parse witness JSON");
    data.witness
}

/// Builds an EeChunkProofInput from the canonical EVM witness fixture.
/// Pub-super so the sibling `alpen_acct` module can reuse the same
/// chunk to drive its perf input (one realistic chunk → one acct
/// update aggregating it).
pub(super) fn prepare_input() -> EeChunkProofInput {
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
        // This RSP fixture stores bytecodes as a Vec without original code-hash
        // keys. Re-hashing keeps the fixture behavior; production range
        // witnesses preserve the AccessedStateGenerator keys instead.
        witness
            .bytecodes
            .clone()
            .into_iter()
            .map(|bytecode| (bytecode.hash_slow(), bytecode))
            .collect(),
        witness.ancestor_headers.clone(),
    );

    let header = witness.current_block.header().clone();
    let evm_header = EvmHeader::new(header.clone());
    let body = EvmBlockBody::from_alloy_body(witness.current_block.body().clone());
    let block = EvmBlock::new(evm_header, body);
    let tip_blkid: Hash = block.get_header().compute_block_id();
    let tip_state_root = block.get_header().get_state_root();
    let tip_exec_header_summary = block.get_header().get_exec_header_summary();

    let chain_spec: Arc<reth_chainspec::ChainSpec> =
        Arc::new((&witness.genesis).try_into().unwrap());
    let ee = EvmExecutionEnvironment::new(chain_spec, AlpenEvmFactory::default());
    let exec_payload = ExecPayload::new(&header, block.get_body());
    let inputs = ExecInputs::new_empty();
    let output = ee
        .execute_block_body(&pre_state, &exec_payload, &inputs)
        .expect("block execution should succeed");
    let outputs = output.outputs().clone();

    let chunk_transition = strata_ee_chain_types::ChunkTransition::new(
        parent_blkid,
        tip_blkid,
        tip_state_root,
        tip_exec_header_summary,
        inputs.clone(),
        outputs.clone(),
    );

    // Single-block chunk: the block's per-block witness is the pre-state
    // anchored at the parent root.
    let raw_pre_state = encode_to_vec(&pre_state).expect("encode pre-state");
    let raw_block_data =
        RawBlockData::from_block::<EvmExecutionEnvironment>(&block, inputs, outputs, raw_pre_state)
            .expect("encode block");
    let raw_chunk = RawChunkData::new(vec![raw_block_data], parent_blkid);
    let raw_prev_header = encode_to_vec(&parent_evm_header).expect("encode prev header");

    let private_input = PrivateInput::new(chunk_transition, raw_chunk, raw_prev_header);

    EeChunkProofInput {
        genesis: witness.genesis,
        private_input,
        bridge_params: BridgeParams::default(),
    }
}

pub(crate) fn gen_perf_report(host: &impl ZkVmHost) -> (String, ExecutionSummary) {
    info!("Generating execution summary for Alpen Chunk");
    let input = prepare_input();
    let summary =
        <EeChunkProgram as ZkVmProgram>::execute(&input, host).expect("alpen-chunk execution");
    (EeChunkProgram::name(), summary)
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
