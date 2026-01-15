//! Inner proof (chunk proof) guest logic.
//!
//! Executes and verifies EVM blocks within a single chunk.

use std::sync::Arc;

use reth_chainspec::ChainSpec;
use rsp_primitives::genesis::Genesis;
use ssz::Decode;
use strata_ee_acct_runtime::{
    SharedPrivateInput, UpdateTransitionData, verify_and_apply_update_transition,
};
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_evm_ee::EvmExecutionEnvironment;
use strata_identifiers::Hash;
use strata_snark_acct_types::ProofState;
use tree_hash::{Sha256Hasher, TreeHash};
use zkaleido::ZkVmEnv;

use crate::{guest_builder::build_commit_segments, types::ChunkProofOutput};

/// Helper to read count-prefixed vector of buffers.
fn read_counted_bufs(zkvm: &impl ZkVmEnv) -> Vec<Vec<u8>> {
    let count_buf = zkvm.read_buf();
    let count = u32::from_le_bytes(count_buf.try_into().expect("count must be 4 bytes")) as usize;
    (0..count).map(|_| zkvm.read_buf()).collect()
}

/// Helper to read [`ExecBlockPackage`] instances.
fn read_exec_block_packages(zkvm: &impl ZkVmEnv) -> Vec<ExecBlockPackage> {
    read_counted_bufs(zkvm)
        .iter()
        .map(|buf| ExecBlockPackage::from_ssz_bytes(buf).expect("Failed to decode package"))
        .collect()
}

/// Processes a chunk proof in the zkVM guest.
pub fn process_chunk_proof(zkvm: &impl ZkVmEnv) {
    let mut astate =
        EeAccountState::from_ssz_bytes(&zkvm.read_buf()).expect("Failed to decode EeAccountState");
    let prev_proof_state =
        ProofState::from_ssz_bytes(&zkvm.read_buf()).expect("Failed to decode ProofState");
    let update_transition = UpdateTransitionData::from_ssz_bytes(&zkvm.read_buf())
        .expect("Failed to decode UpdateTransitionData");

    verify_proof_state_matches(&astate, prev_proof_state.inner_state());

    let coinputs = read_counted_bufs(zkvm);
    let exec_block_packages = read_exec_block_packages(zkvm);
    let raw_blocks = read_counted_bufs(zkvm);

    // Assert lengths match
    assert_eq!(
        exec_block_packages.len(),
        raw_blocks.len(),
        "Block packages and raw blocks length mismatch"
    );

    let raw_prev_header = zkvm.read_buf();
    let raw_partial_pre_state = zkvm.read_buf();
    let genesis: Genesis = zkvm.read_serde();

    let commit_segments = build_commit_segments(&exec_block_packages, &raw_blocks)
        .expect("Failed to build commit segments");
    let chain_spec: Arc<ChainSpec> = Arc::new((&genesis).try_into().expect("Invalid genesis"));
    let ee = EvmExecutionEnvironment::new(chain_spec);
    let shared_private =
        SharedPrivateInput::new(commit_segments, raw_prev_header, raw_partial_pre_state);

    verify_and_apply_update_transition(
        &mut astate,
        &update_transition,
        coinputs.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    )
    .expect("Update transition verification failed");

    let (_, new_state, processed_messages, outputs, extra_data) = update_transition.into_parts();
    let chunk_output = ChunkProofOutput::new(
        prev_proof_state,
        new_state,
        processed_messages,
        outputs,
        extra_data,
    );

    zkvm.commit_buf(&ssz::Encode::as_ssz_bytes(&chunk_output));
}

fn verify_proof_state_matches(astate: &EeAccountState, expected_inner_state: Hash) {
    let computed_hash = Hash::new(TreeHash::<Sha256Hasher>::tree_hash_root(astate).0);
    assert_eq!(computed_hash, expected_inner_state);
}
