//! ZK-SNARK proof verification for checkpoint data
//!
//! Handles verification of zero-knowledge proofs submitted with checkpoint transactions.

use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::{batch::Checkpoint, hash, proof::RollupVerifyingKey};
use zkaleido::ProofReceipt;

use crate::{CoreOLState, error::*, messages, types::CheckpointProofPublicParameters};

/// Constructs expected public parameters from trusted state and checkpoint data
///
/// This function builds the expected public parameters that should match the
/// ones committed to in the zk-SNARK proof. Parameters are constructed from
/// our own trusted state rather than sequencer input for security.
pub(crate) fn construct_checkpoint_proof_public_parameters(
    state: &CoreOLState,
    checkpoint: &Checkpoint,
) -> Result<CheckpointProofPublicParameters> {
    // [PLACE_HOLDER] => Define the role of auxiliary data in public parameters for checkpoint proof
    let prev_epoch_summary = &state.verified_checkpoint;

    let new_batch_info = checkpoint.batch_info();
    let epoch = new_batch_info.epoch() as u32;

    // Validate epoch progression
    let expected_epoch = (prev_epoch_summary.epoch() + 1) as u32;
    if epoch != expected_epoch {
        return Err(CoreError::InvalidEpoch {
            expected: expected_epoch,
            actual: epoch,
        });
    }

    let new_l2_terminal = *new_batch_info.final_l2_block();

    // Validate L2 block slot progression
    let prev_slot = prev_epoch_summary.terminal().slot();
    let new_slot = new_l2_terminal.slot();
    if new_slot <= prev_slot {
        return Err(CoreError::InvalidL2BlockSlot {
            prev_slot,
            new_slot,
        });
    }

    // Validate L1 block height progression
    let prev_l1_height = prev_epoch_summary.new_l1().height();
    let new_l1_hight = new_batch_info.final_l1_block().height();
    if new_l1_hight <= prev_l1_height {
        return Err(CoreError::InvalidL1BlockHeight(format!(
            "new L1 height {new_l1_hight} must be greater than previous height {prev_l1_height}"
        )));
    }

    // [PLACE_HOLDER]
    // TODO: What is the algorithm for calculating the state_diff_hash based on ASM local state?
    // The current approach using hash::hash_data(checkpoint.sidecar().chainstate()) is a
    // placeholder. Need to implement the proper state diff hashing algorithm.
    let state_diff_hash = hash::raw(checkpoint.sidecar().chainstate());

    // TODO: Verify whether extracting post_state_root from batch_transition().chainstate_transition
    // is the correct method for retrieving the new state.
    let new_state = checkpoint
        .batch_transition()
        .chainstate_transition
        .post_state_root;

    let new_epoch_summary = prev_epoch_summary.create_next_epoch_summary(
        new_l2_terminal,
        *new_batch_info.final_l1_block(),
        new_state,
    );

    let l2_to_l1_msgs = messages::extract_l2_to_l1_messages(checkpoint)?;

    // [PLACE_HOLDER] => Waiting for the design of L1 → L2 messaging system and defining what is
    // the l1_commitment should be and etc.
    let l1_to_l2_msgs_range_commitment_hash = messages::l1_to_l2::compute_rolling_hash(
        vec![], // TODO: fetch actual L1 commitments for this range
        prev_l1_height,
        new_l1_hight,
    )?;

    Ok(CheckpointProofPublicParameters {
        epoch_summary: new_epoch_summary,
        state_diff_hash,
        l2_to_l1_msgs,
        prev_l1_ref: *prev_epoch_summary.new_l1(),
        l1_to_l2_msgs_range_commitment_hash,
    })
}

/// Verifies that the provided checkpoint proof is valid for the verifier key
///
/// This function performs zk-SNARK proof verification using the rollup verifying key.
/// It includes logic for handling empty proofs during development/testing phases.
pub(crate) fn verify_checkpoint_proof(
    checkpoint: &Checkpoint,
    proof_receipt: &ProofReceipt,
    rollup_vk: &RollupVerifyingKey,
) -> Result<()> {
    let _checkpoint_idx = checkpoint.batch_info().epoch();

    // FIXME: we are accepting empty proofs for now (devnet) to reduce dependency on the prover
    // infra.
    #[cfg(feature = "debug-utils")]
    let allow_empty = true;
    #[cfg(not(feature = "debug-utils"))]
    let allow_empty = false;
    let is_empty_proof = proof_receipt.proof().is_empty();
    let accept_empty_proof = is_empty_proof && allow_empty;
    let skip_public_param_check = proof_receipt.public_values().is_empty() && allow_empty;
    let is_non_native_vk = !matches!(rollup_vk, RollupVerifyingKey::NativeVerifyingKey);

    if !skip_public_param_check {
        // TODO: Update here based on asm compatible proof structure
    }

    if accept_empty_proof && is_non_native_vk {
        return Ok(());
    }

    if !allow_empty && is_empty_proof {
        return Err(CoreError::InvalidProof);
    }

    verify_rollup_groth16_proof_receipt(proof_receipt, rollup_vk)
        .map_err(|_| CoreError::InvalidProof)
}
