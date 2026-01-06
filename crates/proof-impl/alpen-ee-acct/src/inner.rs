//! Inner proof (chunk proof) guest logic.
//!
//! Executes and verifies EVM blocks within a single chunk.

use std::sync::Arc;

use reth_chainspec::ChainSpec;
use rsp_primitives::genesis::Genesis;
use ssz::Decode;
use strata_codec::decode_buf_exact;
use strata_ee_acct_runtime::{
    SharedPrivateInput, UpdateTransitionData, verify_and_apply_update_transition,
};
use strata_ee_acct_types::EeAccountState;
use strata_evm_ee::EvmExecutionEnvironment;
use strata_identifiers::Hash;
use strata_snark_acct_types::ProofState;
use tree_hash::{Sha256Hasher, TreeHash};
use zkaleido::ZkVmEnv;

use crate::{
    guest_builder::build_commit_segments_from_blocks,
    types::{BytesList, ChunkProofOutput},
};

/// Processes a chunk proof in the zkVM guest.
pub fn process_chunk_proof(zkvm: &impl ZkVmEnv) {
    let mut astate =
        EeAccountState::from_ssz_bytes(&zkvm.read_buf()).expect("Failed to decode EeAccountState");
    let prev_proof_state =
        ProofState::from_ssz_bytes(&zkvm.read_buf()).expect("Failed to decode ProofState");
    let update_transition = UpdateTransitionData::from_ssz_bytes(&zkvm.read_buf())
        .expect("Failed to decode UpdateTransitionData");

    verify_proof_state_matches(&astate, prev_proof_state.inner_state());

    // TODO: Rethink serialization approach for Vec<Vec<u8>>
    let BytesList(coinputs) =
        decode_buf_exact(&zkvm.read_buf()).expect("Failed to decode coinputs");
    let BytesList(block_bytes) =
        decode_buf_exact(&zkvm.read_buf()).expect("Failed to decode block_bytes");

    let raw_prev_header = zkvm.read_buf();
    let raw_partial_pre_state = zkvm.read_buf();
    let genesis: Genesis = zkvm.read_serde();

    let commit_segments =
        build_commit_segments_from_blocks(block_bytes).expect("Failed to build commit segments");
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
    let computed_hash = Hash::from(TreeHash::<Sha256Hasher>::tree_hash_root(astate).0);
    assert_eq!(
        computed_hash, expected_inner_state,
        "Initial state mismatch"
    );
}
