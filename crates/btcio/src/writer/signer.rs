use std::sync::Arc;

use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_btc_types::TxidExt;
use strata_db_types::types::{BundledPayloadEntry, L1TxEntry};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::{
    builder::{
        attach_reveal_signature, build_and_sign_envelope_txs, build_envelope_txs, EnvelopeData,
        EnvelopeError,
    },
    context::WriterContext,
};
use crate::broadcaster::L1BroadcastHandle;

/// Builds envelope transactions for a payload entry.
///
/// Signs the commit tx with the Bitcoin wallet and caches the result in [`EnvelopeData`].
/// Neither transaction is broadcast yet — both are sent together by
/// [`complete_reveal_and_broadcast`] once the external signer provides the reveal signature.
/// This ensures a cache miss on restart is safe: resetting to `Unsigned` cannot orphan a
/// UTXO because nothing has been broadcast.
pub(crate) async fn create_payload_envelopes<R: Reader + Signer + Wallet>(
    payload_idx: u64,
    payloadentry: &BundledPayloadEntry,
    ctx: Arc<WriterContext<R>>,
) -> Result<EnvelopeData, EnvelopeError> {
    let span = debug_span!(
        "btcio_payload_envelope",
        component = "btcio_writer_signer",
        payload_idx,
    );

    async {
        trace!("Building payload envelope transactions");
        let mut envelope = build_envelope_txs(&payloadentry.payload, ctx.as_ref()).await?;

        let commit_txid = envelope.commit_tx.compute_txid();
        debug!(%commit_txid, "Signing commit transaction with wallet");
        let signed_commit = ctx
            .client
            .sign_raw_transaction_with_wallet(&envelope.commit_tx, None)
            .await
            .map_err(|e| EnvelopeError::SignRawTransaction(e.to_string()))?
            .tx;
        envelope.commit_tx = signed_commit;

        info!(%commit_txid, sighash = %envelope.sighash, "envelope built, commit signed");
        Ok(envelope)
    }
    .instrument(span)
    .await
}

/// Builds envelope transactions, signs both in-process with a temporary keypair, and stores
/// them in the broadcaster DB.
///
/// Used when `CredRule::Unchecked` is configured — no external signer is needed.
/// Returns `(commit_txid, reveal_txid)`.
pub(crate) async fn sign_and_broadcast_payload_envelopes<R: Reader + Signer + Wallet>(
    payload_idx: u64,
    payloadentry: &BundledPayloadEntry,
    ctx: Arc<WriterContext<R>>,
    broadcast_handle: &L1BroadcastHandle,
) -> Result<(Buf32, Buf32), EnvelopeError> {
    let span = debug_span!(
        "btcio_payload_envelope_unchecked",
        component = "btcio_writer_signer",
        payload_idx,
    );

    async {
        let envelope = build_and_sign_envelope_txs(&payloadentry.payload, ctx.as_ref()).await?;

        let cid: Buf32 = envelope.commit_tx.compute_txid().to_buf32();
        broadcast_handle
            .put_tx_entry(cid, L1TxEntry::from_tx(&envelope.commit_tx))
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;

        let rid: Buf32 = envelope.reveal_tx.compute_txid().to_buf32();
        broadcast_handle
            .put_tx_entry(rid, L1TxEntry::from_tx(&envelope.reveal_tx))
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;

        info!(%cid, reveal_txid = %rid, "envelope signed and stored for broadcast");
        Ok((cid, rid))
    }
    .instrument(span)
    .await
}

/// Attaches the external signer's Schnorr signature to the reveal tx and stores both
/// commit and reveal for broadcast.
///
/// Called by the watcher when it sees a `PendingRevealTxSign` entry whose
/// `payload_signature` has been filled by the signer RPC.
pub(crate) async fn complete_reveal_and_broadcast(
    payload_idx: u64,
    envelope: &EnvelopeData,
    signature: &[u8; 64],
    broadcast_handle: &L1BroadcastHandle,
) -> Result<Buf32, EnvelopeError> {
    let span = debug_span!(
        "btcio_payload_reveal",
        component = "btcio_writer_signer",
        payload_idx,
    );

    async {
        // Attach the signature first so that any encoding failure aborts
        // before anything is written to the broadcaster DB.
        let mut reveal_tx = envelope.reveal_tx.clone();
        attach_reveal_signature(
            &mut reveal_tx,
            &envelope.reveal_script,
            &envelope.taproot_spend_info,
            signature,
        )
        .map_err(EnvelopeError::Other)?;

        let cid: Buf32 = envelope.commit_tx.compute_txid().to_buf32();
        broadcast_handle
            .put_tx_entry(cid, L1TxEntry::from_tx(&envelope.commit_tx))
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;

        let rid: Buf32 = reveal_tx.compute_txid().to_buf32();
        broadcast_handle
            .put_tx_entry(rid, L1TxEntry::from_tx(&reveal_tx))
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;

        info!(%cid, reveal_txid = %rid, "commit and reveal stored for broadcast");
        Ok(rid)
    }
    .instrument(span)
    .await
}

#[cfg(test)]
mod test {
    use strata_btc_types::TxidExt;
    use strata_csm_types::L1Payload;
    use strata_db_types::types::{BundledPayloadEntry, L1BundleStatus};
    use strata_l1_txfmt::TagData;
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::{
        test_utils::test_context::get_writer_context,
        writer::test_utils::{get_broadcast_handle, get_envelope_ops},
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_payload_envelopes() {
        let iops = get_envelope_ops();
        let bcast_handle = get_broadcast_handle();
        let ctx = get_writer_context();

        // First insert an unsigned blob
        let tag = TagData::new(1, 1, vec![]).unwrap();
        let payload = L1Payload::new(vec![vec![1; 150]; 1], tag);
        let entry = BundledPayloadEntry::new_unsigned(payload);

        assert_eq!(entry.status, L1BundleStatus::Unsigned);
        assert_eq!(entry.commit_txid, Buf32::zero());
        assert_eq!(entry.reveal_txid, Buf32::zero());

        iops.put_payload_entry_async(0, entry.clone())
            .await
            .unwrap();

        let envelope = create_payload_envelopes(0, &entry, ctx).await.unwrap();

        // Commit tx should not be in broadcast DB yet — deferred until reveal sig arrives
        let cid: Buf32 = envelope.commit_tx.compute_txid().to_buf32();
        let ctx_entry = bcast_handle.get_tx_entry_by_id_async(cid).await.unwrap();
        assert!(ctx_entry.is_none());

        // Sighash should be non-zero
        assert_ne!(envelope.sighash, Buf32::zero());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sign_and_broadcast_payload_envelopes() {
        let iops = get_envelope_ops();
        let bcast_handle = get_broadcast_handle();
        let ctx = get_writer_context();

        let tag = TagData::new(1, 1, vec![]).unwrap();
        let payload = L1Payload::new(vec![vec![1; 150]; 1], tag);
        let entry = BundledPayloadEntry::new_unsigned(payload);

        iops.put_payload_entry_async(0, entry.clone())
            .await
            .unwrap();

        let (cid, rid) = sign_and_broadcast_payload_envelopes(0, &entry, ctx, &bcast_handle)
            .await
            .unwrap();

        // Both txids should be non-zero
        assert_ne!(cid, Buf32::zero());
        assert_ne!(rid, Buf32::zero());

        // Both commit and reveal should be stored in broadcaster DB immediately
        assert!(bcast_handle
            .get_tx_entry_by_id_async(cid)
            .await
            .unwrap()
            .is_some());
        assert!(bcast_handle
            .get_tx_entry_by_id_async(rid)
            .await
            .unwrap()
            .is_some());
    }
}
