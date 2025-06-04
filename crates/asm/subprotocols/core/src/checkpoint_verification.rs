//! Local checkpoint verification logic for the Core ASM subprotocol

use strata_primitives::{batch::Checkpoint, params::RollupParams, proof::RollupVerifyingKey};
use tracing::*;
use zkaleido::{ProofReceipt, PublicValues, ZkVmError, ZkVmResult};

/// Constructs a proof receipt with our own public parameters.
///
/// This function constructs the public parameters from the checkpoint's batch transition
/// rather than trusting any externally provided parameters. This prevents attacks where
/// a malicious sequencer could provide incorrect public parameters.
pub(crate) fn construct_receipt_with_our_params(checkpoint: &Checkpoint) -> ProofReceipt {
    let proof = checkpoint.proof().clone();
    let public_output = checkpoint.batch_transition();
    let public_values =
        PublicValues::new(borsh::to_vec(&public_output).expect("checkpoint: proof output"));
    ProofReceipt::new(proof, public_values)
}

/// Verify that the provided checkpoint proof is valid for the verifier key.
///
/// This implementation constructs the public parameters from our own state and the
/// checkpoint's batch transition, rather than trusting sequencer-provided parameters.
/// This prevents potential attacks where incorrect public parameters could be used.
///
/// # Caution
///
/// If the checkpoint proof is empty, this function returns an `Ok(())`.
pub(crate) fn verify_proof(
    checkpoint: &Checkpoint,
    rollup_params: &RollupParams,
) -> ZkVmResult<()> {
    let rollup_vk = rollup_params.rollup_vk;
    let checkpoint_idx = checkpoint.batch_info().epoch();
    info!(%checkpoint_idx, "verifying proof");

    // Construct proof receipt with our own public parameters
    let proof_receipt = construct_receipt_with_our_params(checkpoint);

    // FIXME: we are accepting empty proofs for now (devnet) to reduce dependency on the prover
    // infra.
    let allow_empty = rollup_params.proof_publish_mode.allow_empty();
    let is_empty_proof = proof_receipt.proof().is_empty();
    let accept_empty_proof = is_empty_proof && allow_empty;
    let is_non_native_vk = !matches!(rollup_vk, RollupVerifyingKey::NativeVerifyingKey(_));

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

    verify_rollup_groth16_proof_receipt(&proof_receipt, &rollup_vk)
}

/// Local implementation of groth16 proof verification to avoid strata_crypto dependency
fn verify_rollup_groth16_proof_receipt(
    proof_receipt: &ProofReceipt,
    rollup_vk: &RollupVerifyingKey,
) -> ZkVmResult<()> {
    match rollup_vk {
        RollupVerifyingKey::Risc0VerifyingKey(vk) => {
            zkaleido_risc0_groth16_verifier::verify_groth16(proof_receipt, vk.as_ref())
        }
        RollupVerifyingKey::SP1VerifyingKey(vk) => {
            zkaleido_sp1_groth16_verifier::verify_groth16(proof_receipt, vk.as_ref())
        }
        // In Native Execution mode, we do not actually generate the proof to verify. Checking
        // public parameters is sufficient.
        RollupVerifyingKey::NativeVerifyingKey(_) => Ok(()),
    }
}
