use strata_asm_common::{
    AsmLogsOracleData, AuxData, AuxInput, CompactMmr64, L1TxIndex, L1TxOracleData, Mmr64,
    SubprotocolId, compute_logs_leaf,
};

/// Verifies the entire auxiliary input for a subprotocol.
pub fn verify_aux_input(
    aux_inputs: &AuxInput,
    mmr_state: &CompactMmr64,
    subprotocol: SubprotocolId,
) -> Result<(), String> {
    let mmr = Mmr64::from_compact(mmr_state);
    for (tx_index, aux_data) in &aux_inputs.data {
        verify_aux_data(aux_data, &mmr, subprotocol, *tx_index)?;
    }
    Ok(())
}

/// This function coordinates verification of different oracle types:
/// - ASM log oracles: Verified against History MMR
/// - L1 transaction oracles: Verified against Bitcoin header & header inclusion
///
/// Each oracle type has its own verification logic since they provide
/// different types of auxiliary data with different validation requirements.
fn verify_aux_data(
    aux_data: &AuxData,
    mmr: &Mmr64,
    subprotocol: SubprotocolId,
    tx_index: L1TxIndex,
) -> Result<(), String> {
    // Verify ASM log oracles
    verify_asm_log_oracle(&aux_data.asm_logs_oracle, mmr).map_err(|e| {
        format!(
            "ASM log verification failed for subprotocol {:?}, tx_index {}: {}",
            subprotocol, tx_index, e
        )
    })?;

    // Verify L1 transaction oracles
    verify_l1_tx_oracle(&aux_data.l1_txs_oracle, mmr).map_err(|e| {
        format!(
            "L1 tx verification failed for subprotocol {:?}, tx_index {}: {}",
            subprotocol, tx_index, e
        )
    })?;

    Ok(())
}

/// Verifies ASM log oracle data against the History MMR.
///
/// This checks that the provided logs for each block are committed
/// in the History MMR by verifying the Merkle proof. This ensures
/// the logs are part of the canonical L1 chain history.
///
/// # Arguments
/// * `log_oracles` - Block logs with MMR inclusion proofs
/// * `mmr` - History MMR to verify against
///
/// # Returns
/// * `Ok(())` if all log proofs are valid
/// * `Err(String)` with details about which proof failed
fn verify_asm_log_oracle(log_oracles: &[AsmLogsOracleData], mmr: &Mmr64) -> Result<(), String> {
    for (idx, block_logs) in log_oracles.iter().enumerate() {
        // Compute the MMR leaf from block hash and logs
        let leaf = compute_logs_leaf(&block_logs.block_hash, &block_logs.logs);

        // Verify the proof against the MMR
        if !mmr.verify(&block_logs.proof, &leaf) {
            return Err(format!(
                "Invalid log proof at index {}, block_hash {}",
                idx, block_logs.block_hash
            ));
        }
    }
    Ok(())
}

/// Verifies L1 transaction oracle data.
///
/// Validation Steps:
/// 1- Tx validity
/// 2- Tx inclusion in claimed block (TODO)
/// 3- Block inclusion in canonical chain via MMR (TODO)
///
/// # Arguments
/// * `tx_oracle` - L1 transaction to verify
///
/// # Returns
/// * `Ok(())` if the transaction is valid
/// * `Err(String)` with details about what failed
fn verify_l1_tx_oracle(_tx_oracle: &L1TxOracleData, _mmr: &Mmr64) -> Result<(), String> {
    // TODO: Implement full verification steps as outlined above.
    Ok(())
}

#[cfg(test)]
mod tests {
    // TODO: Testing verify_asm_log_oracle requires setting up an MMR with known leaves and proofs.
    // These tests would ideally be in integration tests where we can set up the full MMR
    // infrastructure.

    // TODO: Testing verify_l1_tx_oracle after implementation.
}
