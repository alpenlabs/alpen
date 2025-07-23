//! Checkpoint data extraction
//!
//! Handles extraction and parsing of signed checkpoint data from transactions.

use strata_asm_common::TxInputRef;
use strata_primitives::batch::SignedCheckpoint;

use crate::{error::*, parsing::inscription};

/// Extracts signed checkpoint data from transaction
///
/// This function parses the transaction to extract the signed checkpoint,
/// handling both direct parsing and inscription envelope parsing.
///
/// # Arguments
/// * `tx` - The transaction input reference containing checkpoint data
///
/// # Returns
/// The extracted signed checkpoint or parsing error
pub(crate) fn extract_signed_checkpoint(tx: &TxInputRef<'_>) -> Result<SignedCheckpoint> {
    // Parse inscription envelope and validate it
    let envelope_data = inscription::parse_inscription_envelope(tx)?;
    inscription::validate_inscription_envelope(&envelope_data, tx.tag().tx_type())?;

    // Deserialize checkpoint from envelope data
    borsh::from_slice(&envelope_data).map_err(|_| {
        CoreError::MalformedSignedCheckpoint(
            "failed to deserialize checkpoint from inscription".to_string(),
        )
    })
}
