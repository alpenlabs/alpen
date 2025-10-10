use crate::RollupVerifyingKey;
use zkaleido::{ProofReceipt, ZkVmResult, ZkVmVerifier};

pub fn verify_rollup_groth16_proof_receipt(
    proof_receipt: &ProofReceipt,
    rollup_vk: &RollupVerifyingKey,
) -> ZkVmResult<()> {
    match rollup_vk {
        RollupVerifyingKey::SP1VerifyingKey(vk) => ZkVmVerifier::verify(vk, proof_receipt),
        // In Native Execution mode, we do not actually generate the proof to verify. Checking
        // public parameters is sufficient.
        RollupVerifyingKey::NativeVerifyingKey => Ok(()),
    }
}
