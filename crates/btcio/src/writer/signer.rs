use std::sync::Arc;

use bitcoin::{consensus, Transaction};
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_db::types::{BundledPayloadEntry, L1TxEntry};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::{
    builder::{build_envelope_txs, EnvelopeError},
    context::WriterContext,
};
use crate::broadcaster::L1BroadcastHandle;

type BlobIdx = u64;

/// Create envelope transactions corresponding to a [`PayloadEntry`].
///
/// This is used during one of the cases:
/// 1. A new payload intent needs to be signed
/// 2. A signed intent needs to be resigned because somehow its inputs were spent/missing
/// 3. A confirmed block that includes the tx gets reorged
pub(crate) async fn create_and_sign_payload_envelopes<R: Reader + Signer + Wallet>(
    payloadentry: &BundledPayloadEntry,
    broadcast_handle: &L1BroadcastHandle,
    ctx: Arc<WriterContext<R>>,
) -> Result<(Buf32, Buf32), EnvelopeError> {
    trace!("Creating and signing payload envelopes");
    let (commit, reveal) = build_envelope_txs(&payloadentry.payloads, ctx.as_ref()).await?;

    let ctxid = commit.compute_txid();
    debug!(commit_txid = ?ctxid, "Signing commit transaction");
    let signed_commit = ctx
        .client
        .sign_raw_transaction_with_wallet(&commit, None)
        .await
        .map_err(|e| EnvelopeError::SignRawTransaction(e.to_string()))?
        .hex;

    let signed_commit: Transaction = consensus::encode::deserialize_hex(&signed_commit)
        .expect("could not deserialize transaction");
    let cid: Buf32 = signed_commit.compute_txid().into();
    let rid: Buf32 = reveal.compute_txid().into();

    let centry = L1TxEntry::from_tx(&signed_commit);
    let rentry = L1TxEntry::from_tx(&reveal);

    // These don't need to be atomic. It will be handled by writer task if it does not find both
    // commit-reveal txs in db by triggering re-signing.
    let _ = broadcast_handle
        .put_tx_entry(cid, centry)
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    let _ = broadcast_handle
        .put_tx_entry(rid, rentry)
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    Ok((cid, rid))
}

#[cfg(test)]
mod test {
    use strata_db::types::{BundledPayloadEntry, L1BundleStatus};
    use strata_primitives::l1::payload::L1Payload;

    use super::*;
    use crate::{
        test_utils::test_context::get_writer_context,
        writer::test_utils::{get_broadcast_handle, get_envelope_ops},
    };

    #[tokio::test]
    async fn test_create_and_sign_blob_envelopes() {
        let iops = get_envelope_ops();
        let bcast_handle = get_broadcast_handle();
        let ctx = get_writer_context();

        // First insert an unsigned blob
        let payload = L1Payload::new_da([1; 100].to_vec());
        let entry = BundledPayloadEntry::new_unsigned(vec![payload]);

        assert_eq!(entry.status, L1BundleStatus::Unsigned);
        assert_eq!(entry.commit_txid, Buf32::zero());
        assert_eq!(entry.reveal_txid, Buf32::zero());

        iops.put_payload_entry_async(0, entry.clone())
            .await
            .unwrap();

        let (cid, rid) = create_and_sign_payload_envelopes(&entry, bcast_handle.as_ref(), ctx)
            .await
            .unwrap();

        // Check if corresponding txs exist in db
        let ctx = bcast_handle.get_tx_entry_by_id_async(cid).await.unwrap();
        let rtx = bcast_handle.get_tx_entry_by_id_async(rid).await.unwrap();
        assert!(ctx.is_some());
        assert!(rtx.is_some());
    }
}
