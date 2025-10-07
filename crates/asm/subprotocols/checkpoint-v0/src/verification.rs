//! Checkpoint verification logic for checkpoint v0
//!
//! This module implements verification procedures that maintain compatibility
//! with the current checkpoint verification system while incorporating SPS-62
//! concepts where beneficial.
//!
//! NOTE: Leverage the current proof/signature verification pipeline until the predicate framework
//! lands

use strata_asm_common::logging;
use strata_checkpoint_types::{
    verify_signed_checkpoint_sig, BatchTransition, Checkpoint, SignedCheckpoint,
};
use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::proof::RollupVerifyingKey;

use crate::{error::CheckpointV0Error, types::CheckpointV0VerifierState};

/// Main checkpoint processing function
///
/// This processes a checkpoint by verifying its validity and updating the verifier state.
/// It bridges SPS-62 concepts with current checkpoint verification for feature parity.
///
/// NOTE: This maintains compatibility with current checkpoint format while following
/// SPS-62 verification flow concepts
pub fn process_checkpoint_v0(
    state: &mut CheckpointV0VerifierState,
    signed_checkpoint: &SignedCheckpoint,
    current_l1_height: u64,
) -> Result<(), CheckpointV0Error> {
    let checkpoint = signed_checkpoint.checkpoint();
    let epoch = checkpoint.batch_info().epoch();

    if !state.can_accept_epoch(epoch) {
        let expected = state.expected_next_epoch();
        logging::warn!(expected, actual = epoch, "Invalid epoch progression");
        return Err(CheckpointV0Error::InvalidEpoch {
            expected,
            actual: epoch,
        });
    }

    ensure_batch_epochs_consistent(checkpoint)?;
    if !verify_signed_checkpoint_sig(signed_checkpoint, &state.cred_rule) {
        return Err(CheckpointV0Error::InvalidSignature);
    }
    verify_checkpoint_proof(checkpoint, state)?;

    if let Some(previous) = &state.last_checkpoint {
        verify_epoch_continuity(previous, checkpoint)?;
    }

    state.update_with_checkpoint(checkpoint.clone(), current_l1_height);
    logging::info!(epoch, "Successfully verified checkpoint");

    Ok(())
}

fn ensure_batch_epochs_consistent(checkpoint: &Checkpoint) -> Result<(), CheckpointV0Error> {
    let info_epoch = checkpoint.batch_info().epoch();
    let transition_epoch = checkpoint.batch_transition().epoch;
    if info_epoch != transition_epoch {
        return Err(CheckpointV0Error::BatchEpochMismatch {
            info_epoch,
            transition_epoch,
        });
    }
    Ok(())
}

fn verify_checkpoint_proof(
    checkpoint: &Checkpoint,
    state: &CheckpointV0VerifierState,
) -> Result<(), CheckpointV0Error> {
    let proof_receipt = checkpoint.construct_receipt();
    let expected_output = *checkpoint.batch_transition();
    let actual_output: BatchTransition =
        borsh::from_slice(proof_receipt.public_values().as_bytes())
            .map_err(|_| CheckpointV0Error::SerializationError)?;

    if expected_output != actual_output {
        logging::warn!(
            epoch = checkpoint.batch_info().epoch(),
            "Checkpoint proof public values mismatch"
        );
        return Err(CheckpointV0Error::InvalidCheckpointProof);
    }

    let is_empty_proof = proof_receipt.proof().is_empty();
    let allow_empty = matches!(
        state.rollup_verifying_key,
        RollupVerifyingKey::NativeVerifyingKey
    );

    if is_empty_proof {
        if allow_empty {
            logging::warn!(
                epoch = checkpoint.batch_info().epoch(),
                "Accepting empty checkpoint proof"
            );
            return Ok(());
        }

        return Err(CheckpointV0Error::InvalidCheckpointProof);
    }

    if let Err(err) =
        verify_rollup_groth16_proof_receipt(&proof_receipt, &state.rollup_verifying_key)
    {
        logging::warn!("Groth16 verification failed: {err:?}");
        return Err(CheckpointV0Error::InvalidCheckpointProof);
    }

    Ok(())
}

/// Ensure that the previous checkpoint's post state matches the current checkpoint's pre state.
fn verify_epoch_continuity(
    prev_checkpoint: &Checkpoint,
    curr_checkpoint: &Checkpoint,
) -> Result<(), CheckpointV0Error> {
    if prev_checkpoint
        .batch_transition()
        .chainstate_transition
        .post_state_root
        != curr_checkpoint
            .batch_transition()
            .chainstate_transition
            .pre_state_root
    {
        return Err(CheckpointV0Error::StateRootMismatch);
    }

    Ok(())
}
