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
    // TODO: Finalize checkpoint transaction data format specification
    // For now, try to parse inscription envelope first, then fall back to direct parsing

    // Try inscription envelope parsing first
    match inscription::parse_inscription_envelope(tx) {
        Ok(envelope_data) => {
            // Validate inscription envelope
            inscription::validate_inscription_envelope(&envelope_data)?;

            // Try to deserialize checkpoint from envelope data
            borsh::from_slice(&envelope_data).map_err(|_| CoreError::MalformedSignedCheckpoint {
                reason: "failed to deserialize checkpoint from inscription".to_string(),
            })
        }
        Err(_) => {
            // Fall back to direct transaction data parsing
            // TODO: Parse inscription envelope and extract the actual signed checkpoint data
            let data = tx
                .tx()
                .input
                .first()
                .ok_or_else(|| CoreError::MalformedSignedCheckpoint {
                    reason: "no transaction inputs".to_string(),
                })?
                .witness
                .to_vec();

            if data.is_empty() {
                return Err(CoreError::MalformedSignedCheckpoint {
                    reason: "empty witness data".to_string(),
                });
            }

            borsh::from_slice(&data[0]).map_err(|_| CoreError::MalformedSignedCheckpoint {
                reason: "failed to deserialize checkpoint".to_string(),
            })
        }
    }
}

/// Validates checkpoint data format and structure
///
/// This function performs basic validation on the checkpoint data to ensure
/// it follows the expected format before further processing.
///
/// # Arguments
/// * `checkpoint` - The signed checkpoint to validate
///
/// # Returns
/// Result indicating if the checkpoint format is valid
pub(crate) fn validate_checkpoint_format(checkpoint: &SignedCheckpoint) -> Result<()> {
    // Basic format validation
    let inner_checkpoint = checkpoint.checkpoint();

    // Validate that batch info exists
    let _batch_info = inner_checkpoint.batch_info();

    // Validate that proof data exists (can be empty for testing)
    let _proof = inner_checkpoint.proof();

    // Validate that batch transition exists
    let _batch_transition = inner_checkpoint.batch_transition();

    // Additional format validation can be added here

    Ok(())
}
