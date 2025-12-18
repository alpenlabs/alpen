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
use ssz::{Decode, Encode};
use strata_codec::decode_buf_exact;
use strata_ee_acct_runtime::{SharedPrivateInput, verify_and_apply_update_operation};
use strata_ee_acct_types::EeAccountState;
use strata_evm_ee::EvmExecutionEnvironment;
use strata_snark_acct_types::{ProofState, UpdateOperationData, UpdateProofPubParams};
use tree_hash::{Sha256Hasher, TreeHash};
use zkaleido::ZkVmEnv;

use crate::guest_builder::build_commit_segments_from_blocks;

// Guest-side block building
mod guest_builder;

// ZkVmProgram implementation
mod program;

// Input types
mod types;

pub use program::{AlpenEeProofProgram, AlpenEeProofProgramOutput};
pub use types::{AlpenEeProofInput, CommitBlockPackage, EeAccountInit, RuntimeUpdateInput};

/// Processes an Alpen EVM EE account update operation in the zkVM guest.
///
/// This is the main guest entry point that verifies and applies an EE account state
/// transition. It deserializes inputs provided by the host, verifies state consistency,
/// processes the update through the EVM execution environment, and commits the final
/// state as proof output.
///
/// # Execution Flow
///
/// 1. **Deserialize inputs**: Reads Codec-serialized [`EeAccountInit`] and [`RuntimeUpdateInput`]
///    structures, plus genesis configuration
/// 2. **Verify initial state**: Ensures the account state hash matches the claimed previous proof
///    state
/// 3. **Build execution context**: Constructs [`SharedPrivateInput`] from commit segments, headers,
///    and partial state
/// 4. **Apply update**: Calls [`verify_and_apply_update_operation`] which validates the operation
///    and executes EVM blocks
/// 5. **Commit output**: Serializes and commits the complete proof output including state
///    transitions, messages, and metadata
///
/// # Input Encoding
///
/// The host provides three inputs via zkVM buffers:
/// - **Account init** (Codec): Initial account state and previous proof state
/// - **Runtime input** (Codec): Update operation, coinputs, blocks, and EVM state
/// - **Genesis** (Serde): Chain configuration for EVM execution environment
///
/// # Panics
///
/// Panics if any deserialization fails, state verification fails, or the update
/// operation is invalid. These represent proof generation failures.
pub fn process_alpen_ee_proof_update(zkvm: &impl ZkVmEnv) {
    // 1. Deserialize grouped input structures using Codec
    let account_init: EeAccountInit =
        decode_buf_exact(&zkvm.read_buf()).expect("Failed to decode EeAccountInit");
    let runtime_input: RuntimeUpdateInput =
        decode_buf_exact(&zkvm.read_buf()).expect("Failed to decode RuntimeUpdateInput");
    let genesis: Genesis = zkvm.read_serde();

    // 2. Extract and decode individual components from account initialization
    let mut astate = EeAccountState::from_ssz_bytes(account_init.astate_ssz())
        .expect("Failed to decode EeAccountState from SSZ");
    let prev_proof_state = ProofState::from_ssz_bytes(account_init.prev_proof_state_ssz())
        .expect("Failed to decode ProofState from SSZ");

    // Verify that the initial astate matches the claimed prev_proof_state
    verify_proof_state_matches(&astate, &prev_proof_state);

    // 3. Extract components from runtime update input
    let operation = UpdateOperationData::from_ssz_bytes(runtime_input.operation_ssz())
        .expect("Failed to decode UpdateOperationData from SSZ");

    // Build CommitChainSegment from blocks
    let commit_segments =
        build_commit_segments_from_blocks(runtime_input.commit_block_packages().to_vec())
            .expect("Failed to build commit segments from blocks");

    // 4. Create execution environment
    let chain_spec: Arc<ChainSpec> = Arc::new((&genesis).try_into().expect("Invalid genesis"));
    let ee = EvmExecutionEnvironment::new(chain_spec);

    let shared_private = SharedPrivateInput::new(
        commit_segments,
        runtime_input.raw_prev_header().to_vec(),
        runtime_input.raw_partial_pre_state().to_vec(),
    );

    // 5. Verify and apply the update operation
    // This internally verifies the extra_data fields (processed_inputs, etc.)
    verify_and_apply_update_operation(
        &mut astate,
        &operation,
        runtime_input.coinputs().iter().map(|v| v.as_slice()),
        &shared_private,
        &ee,
    )
    .expect("Update verification failed");

    // Build the proof output using the standard UpdateProofPubParams interface
    let proof_output = UpdateProofPubParams::new(
        prev_proof_state,                        // Starting state (cur_state)
        operation.new_state(),                   // Resulting state (new_state)
        operation.processed_messages().to_vec(), // Processed inbox messages
        operation.ledger_refs().clone(),         // Ledger accumulator references
        operation.outputs().clone(),             // Account outputs (messages + transfers)
        operation.extra_data().to_vec(),         // Application-specific extra data
    );

    // Commit the output as SSZ-serialized bytes
    let output_bytes = proof_output.as_ssz_bytes();
    zkvm.commit_buf(&output_bytes);
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
