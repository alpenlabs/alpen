use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::L2ToL1Msg;
use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::{
    batch::{Checkpoint, EpochSummary},
    buf::Buf32,
    hash,
    l1::L1BlockCommitment,
    proof::RollupVerifyingKey,
};
use zkaleido::ProofReceipt;

use crate::{CoreOLState, error::*, utils};

#[derive(BorshSerialize, BorshDeserialize)]
pub(crate) struct CheckpointProofPublicParameters {
    /// New epoch summary.
    pub epoch_summary: EpochSummary,
    /// Hash of the OL state diff.
    pub state_diff_hash: Buf32,
    /// Ordered messages L2 → L1. For now, this only includes the
    /// withdrawal requests.
    pub l2_to_l1_msgs: Vec<L2ToL1Msg>,
    /// Previous L1 commitment or genesis.
    pub prev_l1_ref: L1BlockCommitment,
    /// Commitment to the range of L1 → L2 messages.
    pub l1_to_l2_msgs_range_commitment_hash: Buf32,
}

pub(crate) fn construct_expected_public_parameters(
    state: &CoreOLState,
    checkpoint: &Checkpoint,
) -> Result<CheckpointProofPublicParameters> {
    let prev_epoch_summary = state.verified_checkpoint;

    let new_batch_info = checkpoint.batch_info();
    let epoch = new_batch_info.epoch() as u32;

    // Validate epoch progression
    let expected_epoch = (prev_epoch_summary.epoch() + 1) as u32;
    if epoch != expected_epoch {
        return Err(CoreError::InvalidEpoch);
    }

    let new_l2_terminal = *new_batch_info.final_l2_block();

    // Validate L2 block slot progression
    let prev_slot = prev_epoch_summary.terminal().slot();
    if new_l2_terminal.slot() <= prev_slot {
        return Err(CoreError::InvalidL2BlockSlot);
    }

    // Validate L1 block height progression
    let prev_l1_height = prev_epoch_summary.new_l1().height();
    let new_l1_hight = new_batch_info.final_l1_block().height();
    if new_l1_hight <= prev_l1_height {
        return Err(CoreError::InvalidL1BlockHeight);
    }

    // TODO: What is the algorithm for calculating the state_diff_hash?
    // The current approach using hash::hash_data(checkpoint.sidecar().chainstate()) is a
    // placeholder. Need to implement the proper state diff hashing algorithm.
    let state_diff_hash = hash::raw(checkpoint.sidecar().chainstate());

    // TODO: Verify if extracting post_state_root from batch_transition().chainstate_transition
    // is the correct approach for retrieving the new state.
    let new_state = checkpoint
        .batch_transition()
        .chainstate_transition
        .post_state_root;

    let new_epoch_summary = prev_epoch_summary.create_next_epoch_summary(
        new_l2_terminal,
        *new_batch_info.final_l1_block(),
        new_state,
    );

    // Extract L2→L1 messages from checkpoint's data
    let l2_to_l1_msgs = extract_l2_to_l1_messages(checkpoint)?;

    let l1_to_l2_msgs_range_commitment_hash = utils::compute_rolling_hash(
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

// TODO: Parse the actual batch transition structure to extract withdrawal messages
// This is a placeholder implementation that would need to be replaced with
// proper parsing logic based on the actual BatchTransition structure
fn extract_l2_to_l1_messages(_checkpoint: &Checkpoint) -> Result<Vec<L2ToL1Msg>> {
    // For now, return empty vector as we don't have access to the actual
    // withdrawal data structure in the batch transition

    // In a real implementation, this would:
    // 1. Parse the batch transition to find withdrawal operations
    // 2. Extract destination addresses, amounts, and data
    // 3. Validate withdrawal message format
    // 4. Return properly formatted L2ToL1Msg instances

    Ok(Vec::new())
}

/// Verify that the provided checkpoint proof is valid for the verifier key.
pub(crate) fn verify_proof(
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
    let is_non_native_vk = !matches!(rollup_vk, RollupVerifyingKey::NativeVerifyingKey(_));

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
