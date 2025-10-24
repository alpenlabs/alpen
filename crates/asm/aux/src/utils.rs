use strata_asm_common::{
    AuxData, AuxInput, HistoryMmr, HistoryMmrCompact, L1TxIndex, SubprotocolId, compute_log_leaf,
};

fn verify_aux_data(
    aux_data: &AuxData,
    mmr: &HistoryMmr,
    subprotocol: SubprotocolId,
    tx_index: L1TxIndex,
) -> Result<(), String> {
    for block_logs in &aux_data.asm_log_oracles {
        let leaf = compute_log_leaf(&block_logs.block_hash, &block_logs.logs);
        if !mmr.verify(&block_logs.proof, &leaf) {
            return Err(format!(
                "Invalid log proof for subprotocol {:?}, tx_index {}, block_hash {}",
                subprotocol, tx_index, block_logs.block_hash
            ));
        }
    }
    Ok(())
}

pub fn verify_aux_input(
    aux_inputs: &AuxInput,
    mmr_state: &HistoryMmrCompact,
    subprotocol: SubprotocolId,
) -> Result<(), String> {
    let mmr = HistoryMmr::from_compact(mmr_state);
    for (tx_index, aux_data) in &aux_inputs.data {
        verify_aux_data(aux_data, &mmr, subprotocol, *tx_index)?;
    }
    Ok(())
}
