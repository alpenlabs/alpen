use risc0_circuit_recursion::control_id::{ALLOWED_CONTROL_ROOT, BN254_IDENTITY_CONTROL_ID};
use risc0_groth16::verifying_key;
use risc0_zkp::core::digest::Digest;
use strata_primitives::proof::RollupVerifyingKey;
use zkaleido::{ProofReceipt, ZkVmResult, ZkVmVerifier};
use zkaleido_risc0_groth16_verifier::Risc0Groth16Verifier;
use zkaleido_sp1_groth16_verifier::SP1Groth16Verifier;

/// Verifies a Groth16 proof receipt against the rollup verifying key.
///
/// For RISC0: Uses the static verifying key from `risc0_groth16` and control IDs from
/// `risc0_circuit_recursion`. The `RollupVerifyingKey` contains the image_id.
///
/// For SP1: Uses the static verifying key bytes from `sp1_verifier`. The `RollupVerifyingKey`
/// contains the program_vk_hash.
pub fn verify_rollup_groth16_proof_receipt(
    proof_receipt: &ProofReceipt,
    rollup_vk: &RollupVerifyingKey,
) -> ZkVmResult<()> {
    match rollup_vk {
        RollupVerifyingKey::Risc0VerifyingKey(image_id) => {
            let verifier = Risc0Groth16Verifier::new(
                verifying_key(),
                BN254_IDENTITY_CONTROL_ID,
                ALLOWED_CONTROL_ROOT,
                Digest::from_bytes(*image_id.as_ref()),
            );
            ZkVmVerifier::verify(&verifier, proof_receipt)
        }
        RollupVerifyingKey::SP1VerifyingKey(program_vk_hash) => {
            let verifier =
                SP1Groth16Verifier::load(&sp1_verifier::GROTH16_VK_BYTES, *program_vk_hash.as_ref())
                    .map_err(|e| zkaleido::ZkVmError::ProofVerificationError(e.to_string()))?;
            ZkVmVerifier::verify(&verifier, proof_receipt)
        }
        // In Native Execution mode, we do not actually generate the proof to verify. Checking
        // public parameters is sufficient.
        RollupVerifyingKey::NativeVerifyingKey(_) => Ok(()),
    }
}
