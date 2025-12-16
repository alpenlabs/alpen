//! ETH-EE Account Proof Implementation
//!
//! This crate implements the **guest-side** proof generation for ETH-EE account updates.
//! It provides:
//!
//! - Guest processing logic (`process_eth_ee_acct_update`)
//! - ZkVM program definition (`EthEeAcctProgram`)
//! - Input/Output types (`EthEeAcctInput`, `EthEeAcctOutput`)
//!
//! **Note**: Host-side data fetching logic should be implemented by the application
//! using this crate. See README.md for the expected data provider trait.

use std::sync::Arc;

use reth_chainspec::ChainSpec;
use ssz::Decode;
use strata_codec::decode_buf_exact;
use strata_ee_acct_runtime::{SharedPrivateInput, verify_and_apply_update_operation};
use strata_ee_acct_types::{EeAccountState, UpdateExtraData};
use strata_evm_ee::EvmExecutionEnvironment;
use strata_snark_acct_types::{MessageEntry, OutputMessage, ProofState, UpdateOperationData};
use zkaleido::ZkVmEnv;

use crate::guest_builder::build_commit_segments_from_blocks;

// Borsh serialization implementation
mod borsh_impl;

// Guest-side block building
mod guest_builder;

// ZkVmProgram implementation
pub mod program;

pub use program::{EthEeAcctInput, EthEeAcctOutput, EthEeAcctProgram};

/// Public output committed by the ETH-EE account update proof
#[derive(Clone, Debug)]
pub struct EthEeAcctProofOutput {
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

/// Processes an EE account update operation in the zkVM
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
pub fn process_eth_ee_acct_update(zkvm: &impl ZkVmEnv) {
    // 1. Read raw SSZ bytes from zkVM and decode them
    // Each field is passed as raw bytes using write_buf/read_buf
    let mut astate = EeAccountState::from_ssz_bytes(&zkvm.read_buf())
        .expect("Failed to decode EeAccountState from SSZ");
    let operation = UpdateOperationData::from_ssz_bytes(&zkvm.read_buf())
        .expect("Failed to decode UpdateOperationData from SSZ");

    // Coinputs is Vec<Vec<u8>> so we read it with borsh
    let coinputs: Vec<Vec<u8>> = zkvm.read_borsh();

    // Read number of blocks, then read each block's data
    // Each block is: [exec_block_package (SSZ)][raw_block_body (strata_codec)]
    let num_blocks: u32 = zkvm.read_borsh();
    let mut serialized_blocks = Vec::with_capacity(num_blocks as usize);
    for _ in 0..num_blocks {
        serialized_blocks.push(zkvm.read_buf());
    }

    // Build CommitChainSegment from blocks
    let commit_segments = build_commit_segments_from_blocks(serialized_blocks)
        .expect("Failed to build commit segments from blocks");

    // Already raw bytes
    let raw_prev_header = zkvm.read_buf();
    let raw_partial_pre_state = zkvm.read_buf();

    // Read genesis data for ChainSpec construction
    let genesis: rsp_primitives::genesis::Genesis = zkvm.read_serde();
    let chain_spec: Arc<ChainSpec> = Arc::new((&genesis).try_into().expect("Invalid genesis"));
    let ee = EvmExecutionEnvironment::new(chain_spec);

    let shared_private =
        SharedPrivateInput::new(commit_segments, raw_prev_header, raw_partial_pre_state);

    // Capture state before the update
    let prev_state = compute_proof_state(&astate);

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

    // Capture state after the update
    let final_state = compute_proof_state(&astate);

    // Extract UpdateExtraData from the operation (already verified by runtime)
    let extra_data = decode_buf_exact::<UpdateExtraData>(operation.extra_data())
        .expect("Failed to decode UpdateExtraData");

    // Build the complete public output
    let proof_output = EthEeAcctProofOutput {
        prev_state,
        final_state,
        da_commitments: Vec::new(), // Empty for now, TODO: when do we actually fill it??
        output_messages: operation.outputs().messages().to_vec(),
        input_messages: operation.processed_messages().to_vec(),
        extra_data,
    };

    // Commit the complete output
    zkvm.commit_borsh(&proof_output);
}

/// Compute the ProofState from the EE account state
/// TODO: Implement proper state hashing using SSZ hash_tree_root and compute next_msg_read_idx
/// This should match the verification in verify_acct_state_matches()
fn compute_proof_state(_astate: &EeAccountState) -> ProofState {
    // This is critical for proof correctness!
    // Must use SSZ hash_tree_root once EeAccountState has SSZ support
    // For next_msg_read_idx, need to track message processing progress
    let inner_state_root = [0u8; 32]; // TODO: compute actual hash
    let next_msg_read_idx = 0; // TODO: compute actual index
    ProofState::new(inner_state_root.into(), next_msg_read_idx)
}
