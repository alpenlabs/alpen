//! Checkpoint verification logic for checkpointing v0
//!
//! This module implements verification procedures that maintain compatibility
//! with the current checkpoint verification system while incorporating SPS-62
//! concepts where beneficial.
//!
//! NOTE: This bridges to the current proof verification system until unipred
//! is available, as requested for feature parity.

use strata_asm_common::logging;
use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::{proof::RollupVerifyingKey, batch::SignedCheckpoint};
use zkaleido::{ProofReceipt, PublicValues};

use crate::{
    error::CheckpointV0Error,
    types::{
        CheckpointV0VerifierState, CheckpointV0VerificationParams,
        CheckpointV0VerifyContext, CheckpointV0AuxInput, WithdrawalMessages,
    },
};

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
    verify_context: &CheckpointV0VerifyContext,
    _aux_input: &CheckpointV0AuxInput,
    verif_params: &CheckpointV0VerificationParams,
) -> Result<bool, CheckpointV0Error> {
    let checkpoint = signed_checkpoint.checkpoint();

    // 1. Verify epoch progression
    let epoch = checkpoint.batch_info().epoch();
    if !state.can_accept_epoch(epoch) {
        logging::warn!("Invalid epoch progression: expected {}, got {}",
                       state.current_epoch() + 1, epoch);
        return Ok(false);
    }

    // 2. Verify signature (placeholder - would verify against sequencer pubkey)
    if !verify_checkpoint_signature(signed_checkpoint, &verif_params.sequencer_pubkey)? {
        logging::warn!("Checkpoint signature verification failed");
        return Ok(false);
    }

    // 3. Verify proof using current system
    if !verif_params.skip_proof_verification
        && !verify_checkpoint_proof_current_system(checkpoint, verif_params)? {
            logging::warn!("Checkpoint proof verification failed");
            return Ok(false);
        }

    // 4. Verify state transitions (basic validation)
    if let Some(last_checkpoint) = &state.last_checkpoint {
        if !verify_state_transition(last_checkpoint, checkpoint)? {
            logging::warn!("State transition validation failed");
            return Ok(false);
        }
    }

    // 5. Update state with verified checkpoint
    state.update_with_checkpoint(checkpoint.clone(), verify_context.current_l1_height);

    logging::info!("Successfully verified checkpoint for epoch {}", epoch);
    Ok(true)
}

/// Verify checkpoint signature (placeholder for current system compatibility)
fn verify_checkpoint_signature(
    _signed_checkpoint: &SignedCheckpoint,
    _expected_pubkey: &strata_primitives::buf::Buf32,
) -> Result<bool, CheckpointV0Error> {
    // TODO: Implement actual signature verification
    // For now, accept all signatures for feature parity testing
    // In real implementation, this would:
    // 1. Extract signature and pubkey from signed checkpoint
    // 2. Verify signature against checkpoint data
    // 3. Check pubkey matches expected sequencer key

    Ok(true)
}

/// Verify checkpoint proof using current verification system
///
/// NOTE: This bridges to the current groth16 verifier until unipred is ready
fn verify_checkpoint_proof_current_system(
    checkpoint: &strata_primitives::batch::Checkpoint,
    verif_params: &CheckpointV0VerificationParams,
) -> Result<bool, CheckpointV0Error> {
    let proof = checkpoint.proof();
    if proof.is_empty() {
        // Handle empty proofs for testing/development
        #[cfg(feature = "debug-utils")]
        {
            logging::info!("Accepting empty proof in debug mode");
            return Ok(true);
        }
        #[cfg(not(feature = "debug-utils"))]
        {
            logging::warn!("Rejecting empty proof in production mode");
            return Ok(false);
        }
    }

    // Use actual groth16 verification if verifying key is provided
    if let Some(rollup_vk) = &verif_params.rollup_verifying_key {
        logging::info!("Using groth16 verification with provided verifying key");
        return verify_with_current_groth16_system(checkpoint, rollup_vk);
    }

    // Fallback for configurations without verifying key
    logging::warn!("No verifying key provided - proof verification is placeholder");
    logging::warn!("This is NOT SECURE for production use");
    Ok(true)
}

/// Verify state transition between checkpoints
fn verify_state_transition(
    _prev_checkpoint: &strata_primitives::batch::Checkpoint,
    _curr_checkpoint: &strata_primitives::batch::Checkpoint,
) -> Result<bool, CheckpointV0Error> {
    // TODO: Implement state transition verification
    // This would verify:
    // 1. Chainstate root transition is valid
    // 2. L1/L2 block height progression
    // 3. Epoch progression
    // 4. Other state consistency checks

    // For v0, accept all transitions as placeholder
    Ok(true)
}

/// Extract withdrawal messages from checkpoint (current system compatibility)
///
/// This extracts withdrawal messages that should be forwarded to the bridge subprotocol.
/// Currently, withdrawal data is embedded in the checkpoint sidecar.
pub fn extract_withdrawal_messages(
    checkpoint: &strata_primitives::batch::Checkpoint,
) -> Result<WithdrawalMessages, CheckpointV0Error> {
    // Extract withdrawal messages from checkpoint sidecar
    let sidecar = checkpoint.sidecar();
    let messages = WithdrawalMessages::from_checkpoint_sidecar(sidecar);

    Ok(messages)
}

/// Bridge to current proof verification system with proper types
///
/// This function bridges our verification to the existing groth16 verification
/// system until full unipred integration is available.
fn verify_with_current_groth16_system(
    checkpoint: &strata_primitives::batch::Checkpoint,
    rollup_vk: &RollupVerifyingKey,
) -> Result<bool, CheckpointV0Error> {
    // Convert current checkpoint to format expected by current verifier
    // Note: ProofReceipt::new takes ownership, so clone is required here
    let proof = checkpoint.proof().clone();
    let batch_transition = checkpoint.batch_transition();
    let public_values = PublicValues::new(
        borsh::to_vec(batch_transition)
            .map_err(|_| CheckpointV0Error::SerializationError)?
    );

    let receipt = ProofReceipt::new(proof, public_values);

    match verify_rollup_groth16_proof_receipt(&receipt, rollup_vk) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;
    use strata_primitives::{
        batch::{BatchInfo, BatchTransition, Checkpoint, CheckpointSidecar},
        buf::{Buf32, Buf64},
        l1::L1BlockCommitment,
        l2::L2BlockCommitment,
    };
    use strata_state::batch::ChainstateRootTransition;
    use zkaleido::Proof;

    fn create_test_checkpoint() -> SignedCheckpoint {
        let l1_start = L1BlockCommitment::new(199, Buf32::zero().into());
        let l1_end = L1BlockCommitment::new(200, Buf32::zero().into());
        let l2_start = L2BlockCommitment::new(99, Buf32::zero().into());
        let l2_end = L2BlockCommitment::new(100, Buf32::zero().into());

        let batch_info = BatchInfo::new(
            1, // epoch
            (l1_start, l1_end), // L1 range tuple
            (l2_start, l2_end), // L2 range tuple
        );

        let batch_transition = BatchTransition {
            epoch: 1,
            chainstate_transition: ChainstateRootTransition {
                pre_state_root: Buf32::zero(),
                post_state_root: Buf32::zero(),
            },
            tx_filters_transition: strata_primitives::batch::TxFilterConfigTransition {
                pre_config_hash: Buf32::zero(),
                post_config_hash: Buf32::zero(),
            },
        };

        let checkpoint = Checkpoint::new(
            batch_info,
            batch_transition,
            Proof::new(vec![]),
            CheckpointSidecar::new(vec![1, 2, 3, 4]),
        );

        SignedCheckpoint::new(checkpoint, Buf64::zero())
    }

    #[test]
    fn test_epoch_progression_validation() {
        let mut state = CheckpointV0VerifierState::default();
        let signed_checkpoint = create_test_checkpoint();
        let verify_context = CheckpointV0VerifyContext {
            current_l1_height: 100,
            checkpoint_signer_pubkey: Buf32::zero(),
        };
        let verif_params = CheckpointV0VerificationParams {
            sequencer_pubkey: Buf32::zero(),
            skip_proof_verification: true,
            genesis_l1_block: L1BlockCommitment::new(0, Buf32::zero().into()),
            rollup_verifying_key: None, // No verifying key needed for test
        };
        let aux_input = CheckpointV0AuxInput::default();

        // Should accept epoch 1 when current is 0
        let result = process_checkpoint_v0(
            &mut state,
            &signed_checkpoint,
            &verify_context,
            &aux_input,
            &verif_params
        );

        assert!(result.is_ok());
        assert!(result.unwrap());
        assert_eq!(state.current_epoch(), 1);
    }

    #[test]
    fn test_withdrawal_message_extraction() {
        let signed_checkpoint = create_test_checkpoint();
        let result = extract_withdrawal_messages(signed_checkpoint.checkpoint());

        assert!(result.is_ok());
        let messages = result.unwrap();
        assert_eq!(messages.count, 0); // Empty for now
    }
}
