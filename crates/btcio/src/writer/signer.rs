use std::sync::Arc;

use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_btc_types::TxidExt;
use strata_db_types::types::{BundledPayloadEntry, L1TxEntry};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::{
    builder::{attach_reveal_signature, build_envelope_txs, EnvelopeError, UnsignedEnvelopeData},
    context::WriterContext,
};
use crate::broadcaster::L1BroadcastHandle;

/// Builds envelope transactions for a payload entry.
///
/// Signs the commit tx with the Bitcoin wallet and returns the [`UnsignedEnvelopeData`]
/// whose reveal tx requires a Schnorr signature from the external signer.
///
/// The commit tx is stored in the broadcast DB immediately. The reveal tx is returned
/// unsigned — it will be completed by [`complete_reveal_and_broadcast`] once the signer
/// provides the signature.
pub(crate) async fn create_payload_envelopes<R: Reader + Signer + Wallet>(
    payload_idx: u64,
    payloadentry: &BundledPayloadEntry,
    broadcast_handle: &L1BroadcastHandle,
    ctx: Arc<WriterContext<R>>,
) -> Result<(UnsignedEnvelopeData, Buf32), EnvelopeError> {
    let span = debug_span!(
        "btcio_payload_envelope",
        component = "btcio_writer_signer",
        payload_idx,
    );

    async {
        trace!("Building payload envelope transactions");
        let unsigned = build_envelope_txs(&payloadentry.payload, ctx.as_ref()).await?;

        let commit_txid = unsigned.commit_tx.compute_txid();
        debug!(commit_txid = %commit_txid, "Signing commit transaction with wallet");
        let signed_commit = ctx
            .client
            .sign_raw_transaction_with_wallet(&unsigned.commit_tx, None)
            .await
            .map_err(|e| EnvelopeError::SignRawTransaction(e.to_string()))?
            .tx;
        let cid: Buf32 = signed_commit.compute_txid().to_buf32();

        let centry = L1TxEntry::from_tx(&signed_commit);
        broadcast_handle
            .put_tx_entry(cid, centry)
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;

        info!(
            commit_txid = %cid,
            sighash = %unsigned.sighash,
            "built envelope, commit stored — awaiting reveal signature"
        );
        Ok((unsigned, cid))
    }
    .instrument(span)
    .await
}

/// Attaches the external signer's Schnorr signature to the reveal tx and stores it
/// for broadcast.
///
/// Called by the watcher when it sees a `PendingPayloadSign` entry whose
/// `payload_signature` has been filled by the signer RPC.
pub(crate) async fn complete_reveal_and_broadcast(
    payload_idx: u64,
    unsigned: &UnsignedEnvelopeData,
    signature: &[u8; 64],
    broadcast_handle: &L1BroadcastHandle,
) -> Result<Buf32, EnvelopeError> {
    let span = debug_span!(
        "btcio_payload_reveal",
        component = "btcio_writer_signer",
        payload_idx,
    );

    async {
        let mut reveal_tx = unsigned.reveal_tx.clone();
        attach_reveal_signature(
            &mut reveal_tx,
            &unsigned.reveal_script,
            &unsigned.taproot_spend_info,
            signature,
        )
        .map_err(EnvelopeError::Other)?;

        let rid: Buf32 = reveal_tx.compute_txid().to_buf32();
        let rentry = L1TxEntry::from_tx(&reveal_tx);
        broadcast_handle
            .put_tx_entry(rid, rentry)
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;

        info!(reveal_txid = %rid, "reveal tx signed and stored for broadcast");
        Ok(rid)
    }
    .instrument(span)
    .await
}

#[cfg(test)]
mod test {
    use strata_csm_types::L1Payload;
    use strata_db_types::types::{BundledPayloadEntry, L1BundleStatus};
    use strata_l1_txfmt::TagData;

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

        let (unsigned, cid) = create_payload_envelopes(0, &entry, bcast_handle.as_ref(), ctx)
            .await
            .unwrap();

        // Commit tx should be stored in broadcast DB
        let ctx_entry = bcast_handle.get_tx_entry_by_id_async(cid).await.unwrap();
        assert!(ctx_entry.is_some());

        // Sighash should be non-zero
        assert_ne!(unsigned.sighash, Buf32::zero());
    }
}
