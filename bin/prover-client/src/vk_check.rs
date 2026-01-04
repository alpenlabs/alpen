//! Verification key validation for checkpoint proofs.

use std::fs;

use anyhow::{anyhow, Result};
use hex::encode as hex_encode;
use sp1_sdk::{HashableKey, Prover, ProverClient, SP1VerifyingKey};
use sp1_verifier::GROTH16_VK_BYTES;
use strata_params::RollupParams;
use strata_zkvm_hosts::sp1::ELF_BASE_PATH;
use tracing::info;
use zkaleido_sp1_groth16_verifier::SP1Groth16Verifier;

/// Loads the checkpoint ELF file from the configured ELF_BASE_PATH.
fn load_checkpoint_elf() -> Result<Vec<u8>> {
    let elf_path = format!("{}/{}", ELF_BASE_PATH.as_str(), "guest-checkpoint.elf");
    fs::read(&elf_path).map_err(|e| anyhow!("Failed to read ELF file from {}: {}", elf_path, e))
}

/// Extracts Groth16 VK from SP1 ELF bytes using ProverClient and SP1Groth16Verifier.
fn get_groth16_vk_from_elf(elf_bytes: &[u8]) -> Result<Vec<u8>> {
    // Use ProverClient to setup and get SP1 VK
    let prover = ProverClient::builder().cpu().build();
    let (_pk, vk): (_, SP1VerifyingKey) = prover.setup(elf_bytes);

    // Load Groth16 verifier with the SP1 VK hash to get the Groth16 VK
    let groth16_verifier = SP1Groth16Verifier::load(&GROTH16_VK_BYTES, vk.bytes32_raw())
        .map_err(|e| anyhow!("Failed to load SP1 Groth16 verifier: {}", e))?;

    Ok(groth16_verifier.vk.to_uncompressed_bytes())
}

/// Extracts Groth16 VK from checkpoint predicate condition field in rollup parameters.
fn get_vk_from_params(rollup_params: &RollupParams) -> Vec<u8> {
    let predicate_ref = rollup_params.checkpoint_predicate.as_buf_ref();
    predicate_ref.condition().to_vec()
}

/// Validates that the checkpoint VK in rollup parameters matches the loaded ELF VK.
pub(crate) fn validate_checkpoint_vk(rollup_params: &RollupParams) -> Result<()> {
    // Load checkpoint ELF file
    let elf_bytes = load_checkpoint_elf()?;

    // Extract Groth16 VK from ELF
    let loaded_vk = get_groth16_vk_from_elf(&elf_bytes)?;

    // Extract VK from params
    let params_vk = get_vk_from_params(rollup_params);

    if loaded_vk != params_vk {
        return Err(anyhow!(
            "Checkpoint VK mismatch:\nloaded: {}\nparams: {}",
            hex_encode(&loaded_vk),
            hex_encode(&params_vk)
        ));
    }

    info!("Checkpoint VK validated: {}", hex_encode(&loaded_vk));
    Ok(())
}
