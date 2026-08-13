//! Decoding of stored proof receipts.
//!
//! The database keeps receipts as opaque payloads, so the proving-system decoding lives here
//! with the consumer rather than in the storage layer.

use strata_cli_common::errors::DisplayedError;
use strata_db_types::checkpoint_proof::ProofReceiptEntry;
use zkaleido::ProofReceiptWithMetadata;

/// Decodes a stored receipt payload for display.
pub(crate) fn decode_receipt(
    entry: &ProofReceiptEntry,
) -> Result<ProofReceiptWithMetadata, DisplayedError> {
    ProofReceiptWithMetadata::decode(entry.as_bytes()).map_err(|err| {
        DisplayedError::InternalError(
            "Stored proof receipt is malformed".to_string(),
            Box::new(err.to_string()),
        )
    })
}
