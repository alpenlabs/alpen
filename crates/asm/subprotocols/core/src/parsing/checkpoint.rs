//! Checkpoint data extraction
//!
//! Handles extraction and parsing of signed checkpoint data from transactions.

use strata_asm_common::TxInputRef;
use strata_l1tx::{envelope::parser::parse_envelope_payloads, filter::types::TxFilterConfig};
use strata_primitives::{batch::SignedCheckpoint, l1::payload::L1PayloadType};

use crate::error::*;

/// Extracts signed checkpoint data from transaction using strata-l1tx envelope parsing
/// # Arguments
/// * `tx` - The transaction input reference containing checkpoint data
///
/// # Returns
/// The extracted signed checkpoint or parsing error
pub(crate) fn extract_signed_checkpoint(tx: &TxInputRef<'_>) -> Result<SignedCheckpoint> {
    // TODO: The current implementation of parse_envelope_payloads in strata_l1tx relies on
    // TxFilterConfig but we haven't made a decision to adopt TxFilterConfig in the context of
    // ASM or whether we want to refactor parse_envelope_payloads in strata_l1tx. For now we use
    // a mock TxFilterConfig.
    let filter_config = mock_checkpoint_filter_config();

    // Parse checkpoint envelopes using the same pattern as strata-l1tx
    let checkpoints: Vec<SignedCheckpoint> = tx
        .tx()
        .input
        .iter()
        .flat_map(|inp| {
            inp.witness
                .taproot_leaf_script()
                .and_then(|scr| parse_envelope_payloads(&scr.script.into(), &filter_config).ok())
                .map(|items| {
                    items
                        .into_iter()
                        .filter_map(|item| match *item.payload_type() {
                            L1PayloadType::Checkpoint => {
                                // Deserialize checkpoint from payload data
                                borsh::from_slice(item.data()).ok()
                            }
                            L1PayloadType::Da => None,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect();

    // Return the first valid checkpoint found
    checkpoints.into_iter().next().ok_or_else(|| {
        CoreError::TxParsingError("no valid checkpoint envelope found in transaction".to_string())
    })
}

fn mock_checkpoint_filter_config() -> TxFilterConfig {
    unimplemented!("mock TxFilterConfig for checkpoint parsing")
}
