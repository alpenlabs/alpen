//! ETH-EE Account Proof Implementation
//!
//! This crate implements proof generation for ETH-EE account updates, following the pattern
//! established in other proof-impl crates. It provides:
//!
//! - Data fetching layer (`data_provider`)
//! - ZkVM program definition (`program`)
//! - Guest processing logic (this module)

use std::sync::Arc;

use reth_chainspec::ChainSpec;
use strata_codec::decode_buf_exact;
use strata_ee_acct_runtime::{SharedPrivateInput, verify_and_apply_update_operation};
use strata_ee_acct_types::{CommitChainSegment, EeAccountState, UpdateExtraData};
use strata_evm_ee::EvmExecutionEnvironment;
use strata_snark_acct_types::{MessageEntry, OutputMessage, ProofState, UpdateOperationData};
use zkaleido::ZkVmEnv;

// Host-side module (data fetching and proof input preparation)
#[cfg(all(feature = "host", not(target_os = "zkvm")))]
pub mod data_provider;

#[cfg(all(feature = "host", not(target_os = "zkvm")))]
pub use data_provider::{DataProviderError, EthEeAcctDataProvider, UpdateId, prepare_proof_input};

// Borsh serialization implementation
mod borsh_impl;

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
    // 1. Read raw SSZ bytes from zkVM
    // Each field is passed as raw bytes using write_buf/read_buf
    // TODO: Replace with actual SSZ deserialization once available
    let mut astate = decode_ee_account_state_ssz(&zkvm.read_buf())
        .expect("Failed to decode EeAccountState from SSZ");
    let operation = decode_update_operation_ssz(&zkvm.read_buf())
        .expect("Failed to decode UpdateOperationData from SSZ");

    // Coinputs is Vec<Vec<u8>> so we read it with borsh
    let coinputs: Vec<Vec<u8>> = zkvm.read_borsh();

    // Read number of commit segments, then read each one
    let num_segments: u32 = zkvm.read_borsh();
    let mut commit_segments_ssz = Vec::with_capacity(num_segments as usize);
    for _ in 0..num_segments {
        commit_segments_ssz.push(zkvm.read_buf());
    }
    // 3. Build SharedPrivateInput from components
    let commit_segments: Vec<CommitChainSegment> = commit_segments_ssz
        .iter()
        .map(|bytes| {
            CommitChainSegment::decode(bytes).expect("Failed to decode CommitChainSegment")
        })
        .collect();

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

/// Decode EeAccountState from SSZ bytes
/// TODO: Replace with actual SSZ decode once EeAccountState has SSZ support
fn decode_ee_account_state_ssz(_bytes: &[u8]) -> Result<EeAccountState, String> {
    Err("SSZ decoding for EeAccountState not yet implemented".to_string())
}

/// Decode UpdateOperationData from SSZ bytes
/// TODO: Replace with actual SSZ decode (UpdateOperationData already has SSZ Encode/Decode)
fn decode_update_operation_ssz(_bytes: &[u8]) -> Result<UpdateOperationData, String> {
    Err("SSZ decoding for UpdateOperationData not yet implemented".to_string())
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
    ProofState::new(inner_state_root, next_msg_read_idx)
}
