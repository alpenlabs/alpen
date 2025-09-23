//! Checkpoint verification logic for checkpointing v0
//!
//! This module implements verification procedures that maintain compatibility
//! with the current checkpoint verification system while incorporating SPS-62
//! concepts where beneficial.
//!
//! NOTE: This bridges to the current proof verification system until `predicate` framework
//! is available, as requested for feature parity.

use strata_asm_common::logging;
use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::{
    batch::{verify_signed_checkpoint_sig, BatchTransition, Checkpoint, SignedCheckpoint},
    block_credential::CredRule,
    proof::RollupVerifyingKey,
};

use crate::{error::CheckpointV0Error, types::CheckpointV0VerifierState};

/// Main checkpoint processing function (SPS-62 inspired)
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
    verify_checkpoint_signature(signed_checkpoint, &state.cred_rule)?;
    verify_checkpoint_proof(checkpoint, state)?;

    if let Some(previous) = &state.last_checkpoint {
        verify_state_transition(previous, checkpoint)?;
    }

    state.update_with_checkpoint(checkpoint.clone(), current_l1_height);
    logging::info!(epoch, "Successfully verified checkpoint");

    Ok(())
}

fn ensure_batch_epochs_consistent(checkpoint: &Checkpoint) -> Result<(), CheckpointV0Error> {
    let info_epoch = checkpoint.batch_info().epoch();
    let transition_epoch = checkpoint.batch_transition().epoch;
    if info_epoch != transition_epoch {
        return Err(CheckpointV0Error::StateTransitionError(
            "batch info and transition epochs differ".to_string(),
        ));
    }
    Ok(())
}

fn verify_checkpoint_signature(
    signed_checkpoint: &SignedCheckpoint,
    cred_rule: &CredRule,
) -> Result<(), CheckpointV0Error> {
    if verify_signed_checkpoint_sig(signed_checkpoint, cred_rule) {
        Ok(())
    } else {
        Err(CheckpointV0Error::InvalidSignature)
    }
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
        return Err(CheckpointV0Error::InvalidProof);
    }

    let is_empty_proof = proof_receipt.proof().is_empty();
    let allow_empty = state.proof_publish_mode.allow_empty()
        || matches!(
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

        return Err(CheckpointV0Error::InvalidProof);
    }

    if let Err(err) =
        verify_rollup_groth16_proof_receipt(&proof_receipt, &state.rollup_verifying_key)
    {
        logging::warn!("Groth16 verification failed: {err:?}");
        return Err(CheckpointV0Error::InvalidProof);
    }

    Ok(())
}

fn verify_state_transition(
    prev_checkpoint: &Checkpoint,
    curr_checkpoint: &Checkpoint,
) -> Result<(), CheckpointV0Error> {
    let prev_epoch = prev_checkpoint.batch_info().epoch();
    let curr_epoch = curr_checkpoint.batch_info().epoch();

    if curr_epoch != prev_epoch + 1 {
        return Err(CheckpointV0Error::InvalidEpoch {
            expected: prev_epoch + 1,
            actual: curr_epoch,
        });
    }

    if prev_checkpoint
        .batch_transition()
        .chainstate_transition
        .post_state_root
        != curr_checkpoint
            .batch_transition()
            .chainstate_transition
            .pre_state_root
    {
        return Err(CheckpointV0Error::StateTransitionError(
            "L2 state root mismatch between checkpoints".to_string(),
        ));
    }

    Ok(())
}
