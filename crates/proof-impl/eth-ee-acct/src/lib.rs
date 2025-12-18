//! Alpen EVM EE Account Proof Implementation
//!
//! This crate implements the **guest-side** proof generation for Alpen EVM EE account updates.
//! It provides:
//!
//! - Guest processing logic (`process_alpen_ee_proof_update`)
//! - ZkVM program definition (`AlpenEeProofProgram`)
//! - Input/Output types (`AlpenEeProofInput`, `AlpenEeProofProgramOutput`)
//!
//! **Note**: Host-side data fetching logic should be implemented by the application
//! using this crate. See README.md for the expected data provider trait.

use std::sync::Arc;

use reth_chainspec::ChainSpec;
use rsp_primitives::genesis::Genesis;
use ssz::Decode;
use strata_codec::decode_buf_exact;
use strata_ee_acct_runtime::{SharedPrivateInput, verify_and_apply_update_operation};
use strata_ee_acct_types::{EeAccountState, UpdateExtraData};
use strata_evm_ee::EvmExecutionEnvironment;
use strata_snark_acct_types::{MessageEntry, OutputMessage, ProofState, UpdateOperationData};
use tree_hash::{Sha256Hasher, TreeHash};
use zkaleido::ZkVmEnv;

use crate::guest_builder::build_commit_segments_from_blocks;

// Borsh serialization implementation
mod borsh_impl;

// Guest-side block building
mod guest_builder;

// ZkVmProgram implementation
mod program;

pub use guest_builder::CommitBlockPackage;
pub use program::{AlpenEeProofInput, AlpenEeProofProgramOutput, AlpenEeProofProgram};

/// Public output committed by the Alpen EVM EE account update proof
#[derive(Clone, Debug)]
pub struct AlpenEeProofOutput {
    /// Previous account state before the update
    pub prev_state: ProofState,

    /// New account state after the update
    pub final_state: ProofState,

    /// DA commitments (currently empty, will be populated by outer proof)
    pub da_commitments: Vec<[u8; 32]>,

    /// Output messages generated during update
    pub output_messages: Vec<OutputMessage>,

    /// Input messages processed during update
    pub input_messages: Vec<MessageEntry>,

    /// Update metadata (new tip, processed counts)
    pub extra_data: UpdateExtraData,
}

/// Processes an Alpen EVM EE account update operation in the zkVM.
///
/// This function:
/// 1. Reads SSZ-encoded raw bytes from zkVM using read_buf()
/// 2. Manually deserializes SSZ bytes to actual types
/// 3. Builds SharedPrivateInput from components
/// 4. Creates EvmExecutionEnvironment
/// 5. Calls verify_and_apply_update_operation
/// 6. Commits the new state hash as output
///
/// # zkVM I/O Pattern
///
/// The host uses `write_buf()` to pass raw SSZ bytes:
/// ```ignore
/// input_builder.write_buf(&astate_ssz)?;  // Raw SSZ bytes for EeAccountState
/// ```
///
/// The guest uses `read_buf()` to receive raw bytes, then deserializes SSZ:
/// ```ignore
/// let astate_ssz = zkvm.read_buf();  // Get raw SSZ bytes
/// let astate = EeAccountState::from_ssz_bytes(&astate_ssz)?;  // Decode SSZ
/// ```
pub fn process_alpen_ee_proof_update(zkvm: &impl ZkVmEnv) {
    // 1. Read raw SSZ bytes from zkVM and decode them
    // Each field is passed as raw bytes using write_buf/read_buf
    let mut astate = EeAccountState::from_ssz_bytes(&zkvm.read_buf())
        .expect("Failed to decode EeAccountState from SSZ");
    let operation = UpdateOperationData::from_ssz_bytes(&zkvm.read_buf())
        .expect("Failed to decode UpdateOperationData from SSZ");
    let prev_proof_state =
        ProofState::from_ssz_bytes(&zkvm.read_buf()).expect("Failed to decode ProofState from SSZ");

    // Verify that the initial astate matches the claimed prev_proof_state
    verify_proof_state_matches(&astate, &prev_proof_state);

    // Read coinputs (Vec<Vec<u8>>) with SSZ
    let coinputs: Vec<Vec<u8>> = Vec::<Vec<u8>>::from_ssz_bytes(&zkvm.read_buf())
        .expect("Failed to decode coinputs from SSZ");

    // Read number of blocks with SSZ, then read each block's data
    // Each block is a CommitBlockPackage: [exec_block_package (SSZ)][raw_block_body (strata_codec)]
    let num_blocks: u32 =
        u32::from_ssz_bytes(&zkvm.read_buf()).expect("Failed to decode num_blocks from SSZ");
    let mut blocks = Vec::with_capacity(num_blocks as usize);
    for _ in 0..num_blocks {
        blocks.push(CommitBlockPackage::new(zkvm.read_buf()));
    }

    // Build CommitChainSegment from blocks
    let commit_segments = build_commit_segments_from_blocks(blocks)
        .expect("Failed to build commit segments from blocks");

    // Already raw bytes
    let raw_prev_header = zkvm.read_buf();
    let raw_partial_pre_state = zkvm.read_buf();

    // Read genesis data for ChainSpec construction
    let genesis: Genesis = zkvm.read_serde();
    let chain_spec: Arc<ChainSpec> = Arc::new((&genesis).try_into().expect("Invalid genesis"));
    let ee = EvmExecutionEnvironment::new(chain_spec);

    let shared_private =
        SharedPrivateInput::new(commit_segments, raw_prev_header, raw_partial_pre_state);

    // Verify and apply the update operation
    // This internally verifies the extra_data fields (processed_inputs, etc.)
    verify_and_apply_update_operation(
        &mut astate,
        &operation,
        coinputs.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    )
    .expect("Update verification failed");

    // The operation already contains the correct final ProofState
    let final_proof_state = operation.new_state();

    // Extract UpdateExtraData from the operation (already verified by runtime)
    let extra_data = decode_buf_exact::<UpdateExtraData>(operation.extra_data())
        .expect("Failed to decode UpdateExtraData");

    // Build the complete public output
    let proof_output = AlpenEeProofOutput {
        prev_state: prev_proof_state,
        final_state: final_proof_state,
        da_commitments: Vec::new(), // Empty for now, TODO: when do we actually fill it??
        output_messages: operation.outputs().messages().to_vec(),
        input_messages: operation.processed_messages().to_vec(),
        extra_data,
    };

    // Commit the complete output
    zkvm.commit_borsh(&proof_output);
}

/// Verify that the astate hash matches the expected ProofState
///
/// This ensures that the initial account state matches what the proof claims as the starting point.
fn verify_proof_state_matches(astate: &EeAccountState, expected_proof_state: &ProofState) {
    // Compute tree_hash_root of the astate
    let computed_hash = TreeHash::<Sha256Hasher>::tree_hash_root(astate);
    let computed_hash = strata_identifiers::Hash::from(computed_hash.0);

    // Compare with the expected inner_state from ProofState
    if computed_hash != expected_proof_state.inner_state() {
        panic!(
            "Account state hash mismatch: computed {:?}, expected {:?}",
            computed_hash,
            expected_proof_state.inner_state()
        );
    }
}
