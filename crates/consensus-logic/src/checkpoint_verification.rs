//! General handling around checkpoint verification.

use strata_chaintsn::transition::verify_checkpoint_proof;
use strata_checkpoint_types::{BatchTransition, Checkpoint};
use strata_primitives::params::*;
use tracing::*;
use zkaleido::{ProofReceipt, ZkVmError, ZkVmResult};

/// Constructs a receipt from a checkpoint.
///
/// This is here because we want to move `.get_proof_receipt()` out of the
/// checkpoint type itself soon.
pub fn construct_receipt(checkpoint: &Checkpoint) -> ProofReceipt {
    checkpoint.construct_receipt()
}

/// Verify that the provided checkpoint and proof is valid for the verifier key.
///
/// # Caution
///
/// If the checkpoint proof is empty, this function returns an `Ok(())`.
pub fn verify_proof_receipt_against_checkpoint(
    checkpoint: &Checkpoint,
    proof_receipt: &ProofReceipt,
    rollup_params: &RollupParams,
) -> ZkVmResult<()> {
    let checkpoint_idx = checkpoint.batch_info().epoch();
    trace!(%checkpoint_idx, "verifying proof");

    // Do the public parameters check
    let expected_public_output = *checkpoint.batch_transition();
    let actual_public_output: BatchTransition =
        borsh::from_slice(proof_receipt.public_values().as_bytes())
            .map_err(|e| ZkVmError::OutputExtractionError { source: e.into() })?;

    if expected_public_output != actual_public_output {
        dbg!(actual_public_output, expected_public_output);
        return Err(ZkVmError::ProofVerificationError(
            "Public output mismatch during proof verification".to_string(),
        ));
    }

    verify_checkpoint_proof(checkpoint, proof_receipt, rollup_params)
}

#[cfg(test)]
mod tests {
    use strata_primitives::{params::ProofPublishMode, proof::RollupVerifyingKey};
    use strata_test_utils_l2::{gen_params, get_test_signed_checkpoint};
    use zkaleido::{Proof, ProofReceipt, PublicValues, ZkVmError};

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

        let result =
            verify_proof_receipt_against_checkpoint(&checkpoint, &empty_receipt, &rollup_params);

        // Check that the result is an Err containing the OutputExtractionError variant.
        assert!(matches!(
            result,
            Err(ZkVmError::OutputExtractionError { .. })
        ));
    }

    #[test]
    fn test_empty_proof_on_native_mode() {
        let (mut checkpoint, mut rollup_params) = get_test_input();

        // Ensure the mode is Strict for this test
        rollup_params.rollup_vk = RollupVerifyingKey::NativeVerifyingKey;

        let public_values = checkpoint.batch_transition();
        let encoded_public_values = borsh::to_vec(public_values).unwrap();

        // Create a proof receipt with an empty proof and non-empty public values
        let proof_receipt =
            ProofReceipt::new(Proof::new(vec![]), PublicValues::new(encoded_public_values));

        // We have to to make the proof empty a second time because we're sloppy
        // with our receipt handling.
        checkpoint.set_proof(Proof::new(Vec::new()));

        let result =
            verify_proof_receipt_against_checkpoint(&checkpoint, &proof_receipt, &rollup_params);

        // In native mode, there is no proof so it is fine
        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_proof_on_non_native_mode() {
        let (mut checkpoint, rollup_params) = get_test_input();

        // Ensure non native mode
        assert!(!matches!(
            rollup_params.rollup_vk,
            RollupVerifyingKey::NativeVerifyingKey
        ));

        let public_values = checkpoint.batch_transition();
        let encoded_public_values = borsh::to_vec(public_values).unwrap();

        // Create a proof receipt with an empty proof and non-empty public values
        let proof_receipt =
            ProofReceipt::new(Proof::new(vec![]), PublicValues::new(encoded_public_values));

        // We have to to make the proof empty a second time because we're sloppy
        // with our receipt handling.
        checkpoint.set_proof(Proof::new(Vec::new()));

        let result =
            verify_proof_receipt_against_checkpoint(&checkpoint, &proof_receipt, &rollup_params);

        assert!(matches!(
            result,
            Err(ZkVmError::ProofVerificationError { .. })
        ));
    }

    #[test]
    fn test_empty_proof_on_non_native_mode_with_timeout() {
        let (checkpoint, mut rollup_params) = get_test_input();

        // Ensure the mode is Timeout for this test
        rollup_params.proof_publish_mode = ProofPublishMode::Timeout(1_000);

        // Ensure non native mode
        assert!(!matches!(
            rollup_params.rollup_vk,
            RollupVerifyingKey::NativeVerifyingKey
        ));

        let public_values = checkpoint.batch_transition();
        let encoded_public_values = borsh::to_vec(public_values).unwrap();

        // Create a proof receipt with an empty proof and non-empty public values
        let proof_receipt =
            ProofReceipt::new(Proof::new(vec![]), PublicValues::new(encoded_public_values));

        let result =
            verify_proof_receipt_against_checkpoint(&checkpoint, &proof_receipt, &rollup_params);

        eprintln!("verify_proof result {result:?}");
        assert!(result.is_ok());
    }
}
