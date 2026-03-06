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
    BatchInfo, Checkpoint, SignedCheckpoint, verify_signed_checkpoint_sig,
};
use strata_primitives::L1Height;

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
    current_l1_height: L1Height,
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

    if !verify_signed_checkpoint_sig(signed_checkpoint, &state.cred_rule) {
        return Err(CheckpointV0Error::InvalidSignature);
    }
    verify_checkpoint_proof(checkpoint, state)?;

    state.update_with_checkpoint(checkpoint.clone(), current_l1_height);
    logging::info!(epoch, "Successfully verified checkpoint");

    Ok(())
}

fn verify_checkpoint_proof(
    checkpoint: &Checkpoint,
    state: &CheckpointV0VerifierState,
) -> Result<(), CheckpointV0Error> {
    let proof_receipt = checkpoint.construct_receipt();
    let expected_output = checkpoint.batch_info();
    let actual_output: BatchInfo = borsh::from_slice(proof_receipt.public_values().as_bytes())
        .map_err(|_| CheckpointV0Error::SerializationError)?;

    if expected_output != &actual_output {
        logging::warn!(
            epoch = checkpoint.batch_info().epoch(),
            "Checkpoint proof public values mismatch"
        );
        return Err(CheckpointV0Error::InvalidCheckpointProof);
    }

    if let Err(err) = state.predicate.verify_claim_witness(
        proof_receipt.public_values().as_bytes(),
        proof_receipt.proof().as_bytes(),
    ) {
        logging::warn!("Groth16 verification failed: {err:?}");
        return Err(CheckpointV0Error::InvalidCheckpointProof);
    }

    Ok(())
}
