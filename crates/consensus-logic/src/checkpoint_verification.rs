//! General handling around checkpoint verification.

use strata_chaintsn::transition::verify_checkpoint_proof;
use strata_checkpoint_types::{BatchTransition, Checkpoint};
use strata_csm_types::L1Checkpoint;
use strata_params::RollupParams;
use tracing::*;
use zkaleido::ProofReceipt;

use crate::errors::CheckpointError;

/// Verifies if a checkpoint if valid, given the context of a previous checkpoint.
///
/// If this is the first checkpoint we verify, then there is no checkpoint to
/// check against.
///
/// This does NOT check the signature.
// TODO reduce this to actually just passing in the core information we really
// need, not like the height
pub fn verify_checkpoint(
    checkpoint: &Checkpoint,
    prev_checkpoint: Option<&L1Checkpoint>,
    params: &RollupParams,
) -> Result<(), CheckpointError> {
    // First thing obviously is to verify the proof.  No sense in continuing if
    // the proof is invalid.
    let proof_receipt = construct_receipt(checkpoint);
    verify_proof(checkpoint, &proof_receipt, params)?;

    // And check that we're building upon the previous state correctly.
    if let Some(prev) = prev_checkpoint {
        verify_checkpoint_extends(checkpoint, prev, params)?;
    } else {
        // If it's the first checkpoint we want it to be the initial epoch.
        if checkpoint.batch_info().epoch() != 0 {
            return Err(CheckpointError::SkippedGenesis);
        }
    }

    Ok(())
}

/// Verifies that the a checkpoint extends the state of a previous checkpoint.
fn verify_checkpoint_extends(
    checkpoint: &Checkpoint,
    prev: &L1Checkpoint,
    _params: &RollupParams,
) -> Result<(), CheckpointError> {
    let epoch = checkpoint.batch_info().epoch();
    let prev_epoch = prev.batch_info.epoch();
    let last_tsn = prev.batch_transition;
    let tsn = checkpoint.batch_transition();

    // Check that the epoch numbers line up.
    if epoch != prev_epoch + 1 {
        return Err(CheckpointError::Sequencing(epoch, prev_epoch));
    }

    if last_tsn.chainstate_transition.post_state_root != tsn.chainstate_transition.pre_state_root {
        warn!("checkpoint mismatch on L2 state!");
        return Err(CheckpointError::MismatchL2State);
    }

    Ok(())
}

/// Constructs a receipt from a checkpoint.
///
/// This is here because we want to move `.get_proof_receipt()` out of the
/// checkpoint type itself soon.
pub fn construct_receipt(checkpoint: &Checkpoint) -> ProofReceipt {
    checkpoint.construct_receipt()
}

/// Verify that the provided checkpoint proof is valid for the verifier key.
///
/// # Caution
///
/// If the checkpoint proof is empty, this function returns an `Ok(())`.
pub fn verify_proof(
    checkpoint: &Checkpoint,
    proof_receipt: &ProofReceipt,
    rollup_params: &RollupParams,
) -> Result<(), CheckpointError> {
    let checkpoint_idx = checkpoint.batch_info().epoch();
    trace!(%checkpoint_idx, "verifying proof");

    // Do the public parameters check
    let expected_public_output = *checkpoint.batch_transition();
    let actual_public_output: BatchTransition =
        borsh::from_slice(proof_receipt.public_values().as_bytes())
            .map_err(|_| CheckpointError::MalformedTransition)?;

    if expected_public_output != actual_public_output {
        dbg!(actual_public_output, expected_public_output);
        return Err(CheckpointError::TransitionMismatch);
    }

    verify_checkpoint_proof(checkpoint, rollup_params)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use strata_params::ProofPublishMode;
    use strata_predicate::PredicateKey;
    use strata_test_utils_l2::{gen_params, get_test_signed_checkpoint};
    use zkaleido::{Proof, ProofReceipt, PublicValues};

    use super::*;

    fn get_test_input() -> (Checkpoint, RollupParams) {
        let params = gen_params();
        let rollup_params = params.rollup;
        let signed_checkpoint = get_test_signed_checkpoint();
        let checkpoint = signed_checkpoint.checkpoint();

        (checkpoint.clone(), rollup_params)
    }

    #[test]
    fn test_empty_public_values() {
        let (checkpoint, rollup_params) = get_test_input();

        // Explicitly create an empty proof receipt for this test case
        let empty_receipt = ProofReceipt::new(Proof::new(vec![]), PublicValues::new(vec![]));

        let result = verify_proof(&checkpoint, &empty_receipt, &rollup_params);

        // Check that the result is an Err containing the OutputExtractionError variant.
        assert!(matches!(result, Err(CheckpointError::MalformedTransition)));
    }

    #[test]
    fn test_empty_proof_on_native_mode() {
        let (mut checkpoint, mut rollup_params) = get_test_input();

        // Ensure the mode is Strict for this test
        rollup_params.checkpoint_predicate = PredicateKey::always_accept();

        let public_values = checkpoint.batch_transition();
        let encoded_public_values = borsh::to_vec(public_values).unwrap();

        // Create a proof receipt with an empty proof and non-empty public values
        let proof_receipt =
            ProofReceipt::new(Proof::new(vec![]), PublicValues::new(encoded_public_values));

        // We have to to make the proof empty a second time because we're sloppy
        // with our receipt handling.
        checkpoint.set_proof(Proof::new(Vec::new()));

        let result = verify_proof(&checkpoint, &proof_receipt, &rollup_params);

        // In native mode, there is no proof so it is fine
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_proof_on_non_native_mode() {
        let (mut checkpoint, rollup_params) = get_test_input();

        // Ensure non native mode
        assert_eq!(
            rollup_params.checkpoint_predicate,
            PredicateKey::always_accept()
        );

        let public_values = checkpoint.batch_transition();
        let encoded_public_values = borsh::to_vec(public_values).unwrap();

        // Create a proof receipt with an empty proof and non-empty public values
        let proof_receipt =
            ProofReceipt::new(Proof::new(vec![]), PublicValues::new(encoded_public_values));

        // We have to to make the proof empty a second time because we're sloppy
        // with our receipt handling.
        checkpoint.set_proof(Proof::new(Vec::new()));

        let result = verify_proof(&checkpoint, &proof_receipt, &rollup_params);

        assert!(matches!(result, Err(CheckpointError::Proof(_))));
    }

    #[test]
    fn test_empty_proof_on_non_native_mode_with_timeout() {
        let (mut checkpoint, mut rollup_params) = get_test_input();

        // Ensure the mode is Timeout for this test
        rollup_params.proof_publish_mode = ProofPublishMode::Timeout(1_000);

        // Ensure non native mode
        assert_eq!(
            rollup_params.checkpoint_predicate,
            PredicateKey::always_accept()
        );

        let public_values = checkpoint.batch_transition();
        let encoded_public_values = borsh::to_vec(public_values).unwrap();

        // Create a proof receipt with an empty proof and non-empty public values
        let proof_receipt =
            ProofReceipt::new(Proof::new(vec![]), PublicValues::new(encoded_public_values));

        // We have to to make the proof empty a second time because we're sloppy
        // with our receipt handling.
        checkpoint.set_proof(Proof::new(Vec::new()));

        let result = verify_proof(&checkpoint, &proof_receipt, &rollup_params);
        eprintln!("verify_proof result {result:?}");
        assert!(result.is_ok());
    }
}
