//! # Checkpoint transaction extraction helpers for OL DA payload consumption.
//!
//! This is a very simple module that given a raw Bitcoin transaction,
//! can decode its OL DA payload, namely [`OLDaPayloadV1`].

use bitcoin::Transaction;
use strata_asm_common::TxInputRef;
use strata_asm_txs_checkpoint::extract_checkpoint_from_envelope;
use strata_btc_types::RawBitcoinTx;
use strata_l1_txfmt::{MagicBytes, ParseConfig};

use crate::{DaExtractorResult, OLDaPayloadV1, decode_ol_da_payload_bytes};

/// Decodes the OL DA payload from a raw checkpoint transaction.
///
/// It is the caller's responsibility to fetch the raw transaction (e.g., via `btcio` or the
/// consensus layer). This function only handles decoding.
pub fn decode_ol_da_payload(
    raw_tx: RawBitcoinTx,
    magic_bytes: MagicBytes,
) -> DaExtractorResult<OLDaPayloadV1> {
    let tx: Transaction = raw_tx.try_into()?;
    let tag = ParseConfig::new(magic_bytes).try_parse_tx(&tx)?;
    let envelope = extract_checkpoint_from_envelope(&TxInputRef::new(&tx, tag))?;
    let da_payload = decode_ol_da_payload_bytes(envelope.payload.sidecar().ol_state_diff())?;
    Ok(da_payload)
}

#[cfg(test)]
mod tests {
    use bitcoin::{ScriptBuf, Transaction};
    use strata_asm_txs_checkpoint::{CheckpointTxError, OL_STF_CHECKPOINT_TX_TAG};
    use strata_asm_txs_test_utils::create_reveal_transaction_stub;
    use strata_l1_envelope_fmt::parser::parse_envelope_payload;

    use crate::DaExtractorError;

    /// Creates a checkpoint transaction with the given payload, subprotocol, tx type, and secret
    /// key.
    fn make_checkpoint_tx(payload: &[u8]) -> Transaction {
        create_reveal_transaction_stub(payload.to_vec(), &OL_STF_CHECKPOINT_TX_TAG)
    }

    /// Extracts the leaf script from a transaction.
    fn extract_leaf_script(tx: &Transaction) -> Result<ScriptBuf, DaExtractorError> {
        if tx.input.is_empty() {
            return Err(DaExtractorError::CheckpointTxError(
                CheckpointTxError::MissingInputs,
            ));
        }

        tx.input[0]
            .witness
            .taproot_leaf_script()
            .map(|leaf| leaf.script.into())
            .ok_or(DaExtractorError::CheckpointTxError(
                CheckpointTxError::MissingLeafScript,
            ))
    }

    #[test]
    fn test_make_checkpoint_tx_envelope_roundtrip_large_payload() {
        let payload = vec![0xAB; 1_300];
        assert!(payload.len() > 520, "payload must exceed single push limit");

        let tx = make_checkpoint_tx(&payload);

        let script = extract_leaf_script(&tx).expect("extract envelope-bearing leaf script");
        let parsed_payload = parse_envelope_payload(&script).expect("parse envelope payload");
        assert_eq!(parsed_payload, payload);
    }
}
