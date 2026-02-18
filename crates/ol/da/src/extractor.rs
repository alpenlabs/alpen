//! # Checkpoint transaction extraction helpers for OL DA payload consumption.
//!
//! This is a very simple module that given any bitcoin client that implements the [`Reader`] trait,
//! can fetch a checkpoint transaction by [`BitcoinTxid`] and decode its OL DA payload, namely
//! [`OLDaPayloadV1`].

use bitcoin::Transaction;
use bitcoind_async_client::traits::Reader;
use strata_asm_common::TxInputRef;
use strata_asm_txs_checkpoint::extract_signed_checkpoint_from_envelope;
use strata_btc_types::{BitcoinTxid, RawBitcoinTx};
use strata_codec::decode_buf_exact;
use strata_l1_txfmt::{MagicBytes, ParseConfig};

use crate::{DaExtractorError, DaExtractorResult, OLDaPayloadV1};

/// Decodes the OL DA payload from a checkpoint transaction.
pub async fn decode_ol_da_payload<R>(
    reader: &R,
    checkpoint_txid: &BitcoinTxid,
    magic_bytes: MagicBytes,
) -> DaExtractorResult<OLDaPayloadV1>
where
    R: Reader,
{
    let raw_tx = fetch_raw_tx_from_reader(reader, checkpoint_txid).await?;
    let tx: Transaction = raw_tx.try_into()?;
    let tag = ParseConfig::new(magic_bytes).try_parse_tx(&tx)?;
    let signed_checkpoint = extract_signed_checkpoint_from_envelope(&TxInputRef::new(&tx, tag))?;
    let da_payload = decode_buf_exact(signed_checkpoint.inner().sidecar().ol_state_diff())?;
    Ok(da_payload)
}

/// Fetches a raw Bitcoin transaction from a [`Reader`] by [`BitcoinTxid`].
async fn fetch_raw_tx_from_reader<R>(
    reader: &R,
    checkpoint_txid: &BitcoinTxid,
) -> DaExtractorResult<RawBitcoinTx>
where
    R: Reader,
{
    let txid = checkpoint_txid.inner();
    let raw_tx_response = reader
        .get_raw_transaction_verbosity_zero(&txid)
        .await
        .map_err(|_| DaExtractorError::BitcoinTxNotFound(*checkpoint_txid))?;

    Ok(RawBitcoinTx::from(raw_tx_response.0))
}

#[cfg(test)]
mod tests {

    use bitcoin::ScriptBuf;
    use strata_asm_common::test_utils::create_reveal_transaction_stub;
    use strata_asm_txs_checkpoint::{CheckpointTxError, OL_STF_CHECKPOINT_TX_TAG};
    use strata_l1_envelope_fmt::parser::parse_envelope_payload;
    use strata_l1_txfmt::TagData;

    use super::*;
    use crate::DaExtractorError;

    /// Magic bytes for testing purposes.
    const TEST_MAGIC_BYTES: MagicBytes = MagicBytes::new(*b"ALPN");

    /// Creates a checkpoint transaction with the given payload, subprotocol, tx type, and secret
    /// key.
    fn make_checkpoint_tx(payload: &[u8], tag_data: &TagData) -> Transaction {
        let tag_script = ParseConfig::new(TEST_MAGIC_BYTES)
            .encode_script_buf(&tag_data.as_ref())
            .expect("encode tag script");
        create_reveal_transaction_stub(payload.to_vec(), tag_script.into_bytes())
    }

    /// Extracts the leaf script from a transaction.
    fn extract_leaf_script(tx: &Transaction) -> DaExtractorResult<ScriptBuf> {
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

        let tx = make_checkpoint_tx(&payload, &OL_STF_CHECKPOINT_TX_TAG);

        let script = extract_leaf_script(&tx).expect("extract envelope-bearing leaf script");
        let parsed_payload = parse_envelope_payload(&script).expect("parse envelope payload");
        assert_eq!(parsed_payload, payload);
    }
}
