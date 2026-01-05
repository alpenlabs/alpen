//! Verification key validation for checkpoint proofs.

use anyhow::{anyhow, Result};
use hex::encode as hex_encode;
use sp1_sdk::HashableKey;
use sp1_verifier::GROTH16_VK_BYTES;
use strata_zkvm_hosts::sp1::CHECKPOINT_HOST;
use tracing::info;
use zkaleido_sp1_groth16_verifier::SP1Groth16Verifier;

/// Extracts Groth16 VK from CHECKPOINT_HOST.
///
/// Note: CHECKPOINT_HOST is lazily initialized on first access, loading the checkpoint ELF from
/// disk.
pub(crate) fn get_checkpoint_groth16_vk() -> Result<Vec<u8>> {
    let sp1_vk = &CHECKPOINT_HOST.proving_key.vk;
    let groth16_verifier = SP1Groth16Verifier::load(&GROTH16_VK_BYTES, sp1_vk.bytes32_raw())
        .map_err(|e| anyhow!("Failed to load SP1 Groth16 verifier: {}", e))?;
    Ok(groth16_verifier.vk.to_uncompressed_bytes())
}

/// Validates that two verification keys match.
pub(crate) fn validate_checkpoint_vk(loaded_vk: &[u8], params_vk: &[u8]) -> Result<()> {
    if loaded_vk != params_vk {
        return Err(anyhow!(
            "Checkpoint VK mismatch:\nloaded: {}\nparams: {}",
            hex_encode(loaded_vk),
            hex_encode(params_vk)
        ));
    }

    info!("Checkpoint VK validated: {}", hex_encode(loaded_vk));
    Ok(())
}
