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
use strata_ee_acct_runtime::{verify_and_apply_update_operation, SharedPrivateInput};
use strata_ee_acct_types::{CommitChainSegment, EeAccountState};
use strata_evm_ee::EvmExecutionEnvironment;
use strata_snark_acct_types::UpdateOperationData;
use zkaleido::ZkVmEnv;

// Used by ZkVmEnv trait methods (read_borsh, write_borsh)
// Not directly imported but required as dependency
use borsh as _;

// Host-side module (data fetching and proof input preparation)
#[cfg(all(feature = "host", not(target_os = "zkvm")))]
pub mod data_provider;

#[cfg(all(feature = "host", not(target_os = "zkvm")))]
pub use data_provider::{
    prepare_proof_input, DataProviderError, EthEeAcctDataProvider, UpdateId,
};

// ZkVmProgram implementation
pub mod program;

pub use program::{EthEeAcctInput, EthEeAcctOutput, EthEeAcctProgram};

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


    // Verify and apply the update operation
    // This will:
    // - Process messages (deposits, commits, transfers)
    // - Execute chain segments through EVM
    // - Validate state transitions
    // - Apply final changes to account state
    verify_and_apply_update_operation(
        &mut astate,
        &operation,
        coinputs.iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    )
    .expect("Update verification failed");

    // 6. Compute and commit output hash
    let new_state_hash = compute_state_hash(&astate);
    zkvm.commit_borsh(&new_state_hash);
}

/// Decode EeAccountState from SSZ bytes
/// TODO: Replace with actual SSZ decode once EeAccountState has SSZ support
fn decode_ee_account_state_ssz(_bytes: &[u8]) -> Result<EeAccountState, String> {
    Err("SSZ decoding for EeAccountState not yet implemented".to_string())
}

/// Decode UpdateOperationData from SSZ bytes
/// TODO: Replace with actual SSZ decode (UpdateOperationData already has SSZ Encode/Decode)
fn decode_update_operation_ssz(_bytes: &[u8]) -> Result<UpdateOperationData, String> {
    Err("SSZ decoding for UpdateOperationData not yet implemented"
        .to_string())
}


/// Compute the hash of the EE account state
/// TODO: Implement proper state hashing using SSZ hash_tree_root
/// This should match the verification in verify_acct_state_matches()
fn compute_state_hash(_astate: &EeAccountState) -> [u8; 32] {
    // This is critical for proof correctness!
    // Must use SSZ hash_tree_root once EeAccountState has SSZ support
    [0u8; 32]
}
