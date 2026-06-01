use std::sync::Arc;

use bitcoin::{Amount, FeeRate, Transaction};
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_btc_types::TxidExt;
use strata_db_types::types::{
    BundledPayloadEntry, L1TxEntry, L1TxId, L1TxStatus, TxAttempt, TxAttemptStatus, TxNodeId,
    TxNodeKind, TxNodeRecord,
};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::{
    builder::{
        attach_reveal_signature, build_and_sign_envelope_txs, build_envelope_txs, EnvelopeData,
        EnvelopeError,
    },
    context::WriterContext,
};
use crate::{
    broadcaster::L1BroadcastHandle,
    fee_bumper::replacement::{attach_reveal_witness, extract_reveal_witness},
};

fn to_l1_txid(txid: bitcoin::Txid) -> L1TxId {
    L1TxId::from(txid.to_buf32().0)
}

fn to_raw_buf32(txid: L1TxId) -> Buf32 {
    Buf32(txid.0)
}

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
            .map_err(EnvelopeError::SignRawTransaction)?
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
) -> Result<(L1TxId, L1TxId), EnvelopeError> {
    let span = debug_span!(
        "btcio_payload_envelope_unchecked",
        component = "btcio_writer_signer",
        payload_idx,
    );

    async {
        let envelope = build_and_sign_envelope_txs(&payloadentry.payload, ctx.as_ref()).await?;

        let cid = to_l1_txid(envelope.commit_tx.compute_txid());
        broadcast_handle
            .put_tx_entry(
                to_raw_buf32(cid),
                tx_entry_from_envelope(&envelope.commit_tx, &envelope),
            )
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;
        put_tx_node_if_enabled(
            broadcast_handle,
            TxNodeKind::SingleEnvelopeCommit { payload_idx },
            &envelope.commit_tx,
            envelope.fee_rate_sat_vb,
            envelope.commit_fee_sats,
            &envelope,
        )
        .await?;

        let rid = to_l1_txid(envelope.reveal_tx.compute_txid());
        broadcast_handle
            .put_tx_entry(
                to_raw_buf32(rid),
                tx_entry_from_envelope(&envelope.reveal_tx, &envelope),
            )
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;

        info!(?cid, reveal_txid = ?rid, "envelope signed and stored for broadcast");
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
) -> Result<L1TxId, EnvelopeError> {
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

        let cid = to_l1_txid(envelope.commit_tx.compute_txid());
        put_tx_entry_if_missing(broadcast_handle, cid, &envelope.commit_tx, envelope).await?;
        put_tx_node_if_enabled(
            broadcast_handle,
            TxNodeKind::SingleEnvelopeCommit { payload_idx },
            &envelope.commit_tx,
            envelope.fee_rate_sat_vb,
            envelope.commit_fee_sats,
            envelope,
        )
        .await?;

        let rid = to_l1_txid(reveal_tx.compute_txid());
        put_tx_entry_if_missing(broadcast_handle, rid, &reveal_tx, envelope).await?;
        put_tx_node_if_enabled(
            broadcast_handle,
            TxNodeKind::SingleEnvelopeReveal { payload_idx },
            &reveal_tx,
            envelope.fee_rate_sat_vb,
            envelope.reveal_fee_sats,
            envelope,
        )
        .await?;

        info!(?cid, reveal_txid = ?rid, "commit and reveal stored for broadcast");
        Ok(rid)
    }
    .instrument(span)
    .await
}

/// Attaches the external signature to a pending single-envelope reveal replacement.
///
/// Returns `Ok(None)` when no pending replacement exists for this payload.
pub(crate) async fn complete_pending_reveal_replacement(
    payload_idx: u64,
    signature: &[u8; 64],
    broadcast_handle: &L1BroadcastHandle,
) -> Result<Option<L1TxId>, EnvelopeError> {
    let node_id = TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx });
    let Some(mut record) = broadcast_handle
        .get_tx_node(node_id)
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?
    else {
        return Ok(None);
    };
    let Some(previous_signed_attempt) = record.active_attempt().cloned() else {
        return Ok(None);
    };
    if previous_signed_attempt.status != TxAttemptStatus::Active {
        return Ok(None);
    };
    let previous_active_txid = record.active_txid;
    let Some(pending_attempt) = record.pending_signature_attempt().cloned() else {
        return Ok(None);
    };
    let previous_active_entry = broadcast_handle
        .get_tx_entry_by_id_async(to_raw_buf32(previous_active_txid))
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    if matches!(
        previous_active_entry.as_ref().map(|entry| &entry.status),
        Some(L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. })
    ) {
        record.discard_pending_signature_replacement();
        broadcast_handle
            .put_tx_node(record)
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;
        return Ok(None);
    }

    let previous_signed_tx = previous_signed_attempt
        .try_to_tx()
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    let (reveal_script, control_block) =
        extract_reveal_witness(&previous_signed_tx).map_err(|e| EnvelopeError::Other(e.into()))?;

    let mut signed_tx = pending_attempt
        .try_to_tx()
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    attach_reveal_witness(&mut signed_tx, &reveal_script, &control_block, signature)
        .map_err(|e| EnvelopeError::Other(e.into()))?;

    let fee_rate = FeeRate::from_sat_per_vb(pending_attempt.fee_rate_sat_vb).ok_or_else(|| {
        EnvelopeError::Other(anyhow::anyhow!(
            "invalid pending reveal fee rate {}",
            pending_attempt.fee_rate_sat_vb
        ))
    })?;
    let fee_sats = Amount::from_sat(pending_attempt.fee_sats);
    let txid = to_l1_txid(signed_tx.compute_txid());
    let activated = record.activate_pending_signature(&signed_tx, fee_rate, fee_sats);
    if !activated {
        return Ok(None);
    }

    let mut replaced_entry = previous_active_entry.ok_or_else(|| {
        EnvelopeError::Other(anyhow::anyhow!(
            "previous reveal tx entry missing for pending replacement"
        ))
    })?;
    replaced_entry.status = L1TxStatus::Replaced { by: txid };
    broadcast_handle
        .put_tx_entry(
            to_raw_buf32(txid),
            L1TxEntry::from_tx_with_fee_rate(&signed_tx, fee_rate),
        )
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    broadcast_handle
        .update_tx_entry_by_id_async(to_raw_buf32(previous_active_txid), replaced_entry)
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    broadcast_handle
        .put_tx_node(record)
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;

    info!(
        ?txid,
        "pending reveal replacement signed and stored for broadcast"
    );
    Ok(Some(txid))
}

fn tx_entry_from_envelope(tx: &Transaction, envelope: &EnvelopeData) -> L1TxEntry {
    if envelope.fee_bumping_enabled() {
        L1TxEntry::from_tx_with_fee_rate(tx, envelope.fee_rate_sat_vb)
    } else {
        L1TxEntry::from_tx(tx)
    }
}

async fn put_tx_entry_if_missing(
    broadcast_handle: &L1BroadcastHandle,
    txid: L1TxId,
    tx: &Transaction,
    envelope: &EnvelopeData,
) -> Result<(), EnvelopeError> {
    if broadcast_handle
        .get_tx_entry_by_id_async(to_raw_buf32(txid))
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?
        .is_some()
    {
        return Ok(());
    }

    broadcast_handle
        .put_tx_entry(to_raw_buf32(txid), tx_entry_from_envelope(tx, envelope))
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    Ok(())
}

async fn put_tx_node_if_enabled(
    broadcast_handle: &L1BroadcastHandle,
    kind: TxNodeKind,
    tx: &Transaction,
    fee_rate: FeeRate,
    fee_sats: Amount,
    envelope: &EnvelopeData,
) -> Result<(), EnvelopeError> {
    if !envelope.fee_bumping_enabled() {
        return Ok(());
    }

    let node_id = TxNodeId::from_kind(&kind);
    let attempt = TxAttempt::active(tx, fee_rate, fee_sats, 0);
    if let Some(mut record) = broadcast_handle
        .get_tx_node(node_id)
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?
    {
        if record.active_txid == attempt.txid {
            return Ok(());
        }
        record.replace_initial_attempt(attempt);
        broadcast_handle
            .put_tx_node(record)
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;
        return Ok(());
    }

    let record = TxNodeRecord::new(kind, attempt);
    broadcast_handle
        .put_tx_node(record)
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    Ok(())
}

#[cfg(test)]
mod test {
    use strata_config::btcio::{FeeBumpPolicy, FeeBumpingConfig, WriterConfig};
    use strata_csm_types::L1Payload;
    use strata_db_types::types::{
        BundledPayloadEntry, L1BundleStatus, TerminalError, TxNodeId, TxNodeKind,
    };
    use strata_l1_txfmt::TagData;
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::{
        test_utils::{
            test_context::{get_writer_context, get_writer_context_with_client},
            TestBitcoinClient,
        },
        writer::{
            test_utils::{get_broadcast_handle, get_envelope_ops},
            WriterContext,
        },
    };

    fn unsigned_test_entry() -> BundledPayloadEntry {
        let tag = TagData::new(1, 1, vec![]).unwrap();
        let payload = L1Payload::new(vec![vec![1; 150]; 1], tag);
        BundledPayloadEntry::new_unsigned(payload)
    }

    fn get_fee_bumping_writer_context() -> Arc<WriterContext<TestBitcoinClient>> {
        let ctx = get_writer_context();
        Arc::new(WriterContext {
            config: Arc::new(WriterConfig {
                fee_bumping: FeeBumpingConfig {
                    policy: FeeBumpPolicy::Rbf,
                    ..FeeBumpingConfig::default()
                },
                ..(*ctx.config).clone()
            }),
            ..(*ctx).clone()
        })
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_payload_envelopes() {
        let iops = get_envelope_ops();
        let bcast_handle = get_broadcast_handle();
        let ctx = get_writer_context();

        // First insert an unsigned blob
        let entry = unsigned_test_entry();

        assert_eq!(entry.status, L1BundleStatus::Unsigned);
        assert_eq!(entry.commit_txid, L1TxId::zero());
        assert_eq!(entry.reveal_txid, L1TxId::zero());

        iops.put_payload_entry_async(0, entry.clone())
            .await
            .unwrap();

        let envelope = create_payload_envelopes(0, &entry, ctx).await.unwrap();

        // Commit tx should not be in broadcast DB yet — deferred until reveal sig arrives
        let cid = to_l1_txid(envelope.commit_tx.compute_txid());
        let ctx_entry = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(cid))
            .await
            .unwrap();
        assert!(ctx_entry.is_none());

        // Sighash should be non-zero
        assert_ne!(envelope.sighash, Buf32::zero());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sign_and_broadcast_payload_envelopes() {
        let iops = get_envelope_ops();
        let bcast_handle = get_broadcast_handle();
        let ctx = get_writer_context();

        let entry = unsigned_test_entry();

        iops.put_payload_entry_async(0, entry.clone())
            .await
            .unwrap();

        let (cid, rid) = sign_and_broadcast_payload_envelopes(0, &entry, ctx, &bcast_handle)
            .await
            .unwrap();

        // Both txids should be non-zero
        assert_ne!(cid, L1TxId::zero());
        assert_ne!(rid, L1TxId::zero());

        // Both commit and reveal should be stored in broadcaster DB immediately
        assert!(bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(cid))
            .await
            .unwrap()
            .is_some());
        assert!(bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(rid))
            .await
            .unwrap()
            .is_some());
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn test_create_payload_envelopes_preserves_not_enough_utxos() {
        let client = Arc::new(TestBitcoinClient::new(1).with_utxo_amount_sats(1000));
        let ctx = get_writer_context_with_client(client);
        let entry = unsigned_test_entry();

        let err = create_payload_envelopes(0, &entry, ctx).await.unwrap_err();

        assert!(matches!(err, EnvelopeError::NotEnoughUtxos(_, 1000)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sign_and_broadcast_payload_envelopes_preserves_not_enough_utxos() {
        let client = Arc::new(TestBitcoinClient::new(1).with_utxo_amount_sats(1000));
        let ctx = get_writer_context_with_client(client);
        let bcast_handle = get_broadcast_handle();
        let entry = unsigned_test_entry();

        let err = sign_and_broadcast_payload_envelopes(0, &entry, ctx, &bcast_handle)
            .await
            .unwrap_err();

        assert!(matches!(err, EnvelopeError::NotEnoughUtxos(_, 1000)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sign_and_broadcast_payload_envelopes_persists_rbf_metadata() {
        let bcast_handle = get_broadcast_handle();
        let ctx = get_fee_bumping_writer_context();

        let tag = TagData::new(1, 1, vec![]).unwrap();
        let payload = L1Payload::new(vec![vec![1; 150]; 1], tag);
        let entry = BundledPayloadEntry::new_unsigned(payload);

        let (cid, rid) = sign_and_broadcast_payload_envelopes(7, &entry, ctx, &bcast_handle)
            .await
            .unwrap();

        let commit_entry = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(cid))
            .await
            .unwrap()
            .expect("commit entry must exist");
        let reveal_entry = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(rid))
            .await
            .unwrap()
            .expect("reveal entry must exist");

        assert!(commit_entry.rbf.is_some());
        assert!(reveal_entry.rbf.is_some());
        assert!(bcast_handle
            .get_tx_node(TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeCommit {
                payload_idx: 7
            }))
            .await
            .unwrap()
            .is_some());
        assert!(bcast_handle
            .get_tx_node(TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal {
                payload_idx: 7
            }))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_complete_reveal_and_broadcast_creates_reveal_tx_node() {
        let bcast_handle = get_broadcast_handle();
        let ctx = get_fee_bumping_writer_context();
        let entry = unsigned_test_entry();
        let envelope = create_payload_envelopes(7, &entry, ctx).await.unwrap();
        let signature = [1u8; 64];

        complete_reveal_and_broadcast(7, &envelope, &signature, &bcast_handle)
            .await
            .unwrap();

        assert!(bcast_handle
            .get_tx_node(TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal {
                payload_idx: 7
            }))
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_put_tx_node_refreshes_stale_terminal_record() {
        let bcast_handle = get_broadcast_handle();
        let ctx = get_fee_bumping_writer_context();
        let entry = unsigned_test_entry();
        let mut envelope = create_payload_envelopes(7, &entry, ctx).await.unwrap();
        envelope.set_fee_bumping_enabled(true);
        let kind = TxNodeKind::SingleEnvelopeCommit { payload_idx: 7 };

        put_tx_node_if_enabled(
            &bcast_handle,
            kind.clone(),
            &envelope.commit_tx,
            envelope.fee_rate_sat_vb,
            envelope.commit_fee_sats,
            &envelope,
        )
        .await
        .unwrap();
        let node_id = TxNodeId::from_kind(&kind);
        let mut stale_record = bcast_handle
            .get_tx_node(node_id)
            .await
            .unwrap()
            .expect("tx-node exists");
        stale_record.set_terminal_error(TerminalError::WalletInsufficient);
        bcast_handle.put_tx_node(stale_record).await.unwrap();

        let mut replacement_tx = envelope.commit_tx.clone();
        replacement_tx.output[0].value -= Amount::from_sat(1);
        let replacement_txid = to_l1_txid(replacement_tx.compute_txid());
        put_tx_node_if_enabled(
            &bcast_handle,
            kind,
            &replacement_tx,
            envelope.fee_rate_sat_vb,
            envelope.commit_fee_sats,
            &envelope,
        )
        .await
        .unwrap();

        let refreshed = bcast_handle
            .get_tx_node(node_id)
            .await
            .unwrap()
            .expect("tx-node exists");
        assert_eq!(refreshed.active_txid, replacement_txid);
        assert_eq!(refreshed.terminal_error, None);
        assert_eq!(refreshed.attempts.len(), 1);
    }
}
