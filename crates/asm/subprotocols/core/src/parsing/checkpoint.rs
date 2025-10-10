//! Checkpoint data extraction
//!
//! Handles extraction and parsing of signed checkpoint data from transactions.

use strata_asm_common::TxInputRef;
use strata_checkpoint_types::SignedCheckpoint;
use strata_l1tx::{TxFilterConfig, filter::checkpoint::parse_valid_checkpoint_envelope};

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

    parse_valid_checkpoint_envelope(tx.tx(), &filter_config).ok_or_else(|| {
        CoreError::TxParsingError("no valid checkpoint envelope found in transaction".to_string())
    })
}

fn mock_checkpoint_filter_config() -> TxFilterConfig {
    unimplemented!("mock TxFilterConfig for checkpoint parsing")
}
