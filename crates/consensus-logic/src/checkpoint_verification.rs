//! General handling around checkpoint verification.

use strata_crypto::groth16_verifier::verify_rollup_groth16_proof_receipt;
use strata_primitives::{params::*, proof::RollupVerifyingKey};
use strata_state::batch::*;
use tracing::*;
use zkaleido::{ProofReceipt, ZkVmError, ZkVmResult};

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
    prev_checkpoint_commitment: Option<&CheckpointCommitment>,
    params: &RollupParams,
) -> Result<(), CheckpointError> {
    // First thing obviously is to verify the proof.  No sense in continuing if
    // the proof is invalid.
    let proof_receipt = construct_receipt(checkpoint);
    verify_proof(checkpoint, &proof_receipt, params)?;

    // And check that we're building upon the previous state correctly.
    if let Some(prev) = prev_checkpoint_commitment {
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
    prev_commitment: &CheckpointCommitment,
    _params: &RollupParams,
) -> Result<(), CheckpointError> {
    let epoch = checkpoint.batch_info().epoch();
    let prev_epoch = prev_commitment.batch_info().epoch();
    let last_tsn = prev_commitment.batch_transition();
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
    #[allow(deprecated)]
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
) -> ZkVmResult<()> {
    let rollup_vk = rollup_params.rollup_vk;
    let checkpoint_idx = checkpoint.batch_info().epoch();
    info!(%checkpoint_idx, "verifying proof");

    // FIXME: we are accepting empty proofs for now (devnet) to reduce dependency on the prover
    // infra.
    let allow_empty = rollup_params.proof_publish_mode.allow_empty();
    let is_empty_proof = proof_receipt.proof().is_empty();
    let accept_empty_proof = is_empty_proof && allow_empty;
    let skip_public_param_check = proof_receipt.public_values().is_empty() && allow_empty;
    let is_non_native_vk = !matches!(rollup_vk, RollupVerifyingKey::NativeVerifyingKey(_));

    if !skip_public_param_check {
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
    }

    if accept_empty_proof && is_non_native_vk {
        warn!(%checkpoint_idx, "verifying empty proof as correct");
        return Ok(());
    }

    if !allow_empty && is_empty_proof {
        return Err(ZkVmError::ProofVerificationError(format!(
            "Empty proof received for checkpoint {checkpoint_idx}, which is not allowed in strict proof mode. \
            Check `proof_publish_mode` in rollup_params; set it to a non-strict mode (e.g., `timeout`) to accept empty proofs."
        )));
    }

    verify_rollup_groth16_proof_receipt(proof_receipt, &rollup_vk)
}

#[cfg(test)]
mod tests {
    use strata_primitives::params::ProofPublishMode;
    use strata_test_utils::l2::{gen_params, get_test_signed_checkpoint};
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
    fn test_empty_proof_and_empty_public_values_on_strict_mode() {
        let (checkpoint, mut rollup_params) = get_test_input();

        // Ensure the mode is Strict for this test
        rollup_params.proof_publish_mode = ProofPublishMode::Strict;

        // Explicitly create an empty proof receipt for this test case
        let empty_receipt = ProofReceipt::new(Proof::new(vec![]), PublicValues::new(vec![]));

        let result = verify_proof(&checkpoint, &empty_receipt, &rollup_params);

        // Check that the result is an Err containing the OutputExtractionError variant.
        assert!(matches!(
            result,
            Err(ZkVmError::OutputExtractionError { .. })
        ));
    }

    #[test]
    fn test_empty_proof_and_non_empty_public_values_on_strict_mode() {
        let (checkpoint, mut rollup_params) = get_test_input();

        // Ensure the mode is Strict for this test
        rollup_params.proof_publish_mode = ProofPublishMode::Strict;

        let public_values = checkpoint.batch_transition();
        let encoded_public_values = borsh::to_vec(public_values).unwrap();

        // Create a proof receipt with an empty proof and non-empty public values
        let proof_receipt =
            ProofReceipt::new(Proof::new(vec![]), PublicValues::new(encoded_public_values));

        let result = verify_proof(&checkpoint, &proof_receipt, &rollup_params);

        // Check that the result is an Err containing the ProofVerificationError variant
        // and that the error message matches the expected format for empty proofs in strict mode.
        assert!(
            matches!(result, Err(ZkVmError::ProofVerificationError(msg)) if msg.contains("Empty proof received for checkpoint") && msg.contains("which is not allowed in strict proof mode"))
        );
    }

    #[test]
    fn test_empty_proof_on_timeout_mode_with_non_native_vk() {
        let (checkpoint, mut rollup_params) = get_test_input();

        // Ensure the mode is Timeout for this test
        rollup_params.proof_publish_mode = ProofPublishMode::Timeout(1_000);

        // Ensure the VK is non-native for this test
        rollup_params.rollup_vk = RollupVerifyingKey::SP1VerifyingKey(
            "0x00b01ae596b4e51843484ff71ccbd0dd1a030af70b255e6b9aad50b81d81266f"
                .parse()
                .unwrap(),
        );

        let empty_receipt = ProofReceipt::new(Proof::new(vec![]), PublicValues::new(vec![]));

        let result = verify_proof(&checkpoint, &empty_receipt, &rollup_params);

        assert!(result.is_ok());
    }

    #[test]
    fn test_empty_proof_on_timeout_mode_with_native_vk() {
        let (checkpoint, mut rollup_params) = get_test_input();

        // Ensure the mode is Timeout for this test
        rollup_params.proof_publish_mode = ProofPublishMode::Timeout(1_000);

        // Ensure the VK is native for this test
        rollup_params.rollup_vk = RollupVerifyingKey::NativeVerifyingKey(
            "0000000000000000000000000000000000000000000000000000000000000000"
                .parse()
                .unwrap(),
        );

        let empty_receipt = ProofReceipt::new(Proof::new(vec![]), PublicValues::new(vec![]));

        let result = verify_proof(&checkpoint, &empty_receipt, &rollup_params);

        assert!(result.is_ok());
    }
}
