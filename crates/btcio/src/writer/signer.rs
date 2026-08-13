use std::sync::Arc;

use bitcoin::{secp256k1::XOnlyPublicKey, Amount, FeeRate, Transaction};
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use strata_btc_types::TxidExt;
use strata_db_types::{
    common::L1TxId,
    fee_bump::{TerminalError, TxAttempt, TxAttemptStatus, TxNodeId, TxNodeKind, TxNodeRecord},
    l1_broadcast::{L1TxEntry, L1TxStatus},
    l1_writer::BundledPayloadEntry,
};
use strata_primitives::buf::Buf32;
use tracing::*;

use super::{
    builder::{
        attach_reveal_signature, build_and_sign_envelope_txs, build_envelope_txs, EnvelopeData,
        EnvelopeError,
    },
    context::{EnvelopeSigningMode, WriterContext},
};
use crate::{
    broadcaster::L1BroadcastHandle,
    tx_attempt::{attempt_parts, TxAttemptExt},
    tx_entry::L1TxEntryExt,
    writer::replacement::build::{
        attach_reveal_witness, extract_reveal_witness, reveal_script_pubkey,
    },
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
    envelope_pubkey: XOnlyPublicKey,
) -> Result<EnvelopeData, EnvelopeError> {
    let span = debug_span!(
        "btcio_payload_envelope",
        component = "btcio_writer_signer",
        payload_idx,
    );

    async {
        trace!("Building payload envelope transactions");
        let mut envelope =
            build_envelope_txs(&payloadentry.payload, ctx.as_ref(), envelope_pubkey).await?;

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
/// Used when no external signer is required.
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
                L1TxEntry::from_tx_with_fee(
                    &envelope.commit_tx,
                    envelope.fee_rate,
                    envelope.commit_fee,
                ),
            )
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;
        put_tx_node(
            broadcast_handle,
            TxNodeKind::SingleEnvelopeCommit { payload_idx },
            &envelope.commit_tx,
            envelope.fee_rate,
            envelope.commit_fee,
        )
        .await?;

        let rid = to_l1_txid(envelope.reveal_tx.compute_txid());
        broadcast_handle
            .put_tx_entry(
                to_raw_buf32(rid),
                L1TxEntry::from_tx_with_fee(
                    &envelope.reveal_tx,
                    envelope.fee_rate,
                    envelope.reveal_fee,
                ),
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
        put_tx_entry_if_missing(
            broadcast_handle,
            cid,
            &envelope.commit_tx,
            envelope.commit_fee,
            envelope,
        )
        .await?;
        put_tx_node(
            broadcast_handle,
            TxNodeKind::SingleEnvelopeCommit { payload_idx },
            &envelope.commit_tx,
            envelope.fee_rate,
            envelope.commit_fee,
        )
        .await?;

        // Record the reveal node before the broadcast entry: the commit fee bumper keys its
        // "has a reveal been published yet" check off this record, so it must never lag behind
        // the broadcaster.
        let rid = to_l1_txid(reveal_tx.compute_txid());
        put_tx_node(
            broadcast_handle,
            TxNodeKind::SingleEnvelopeReveal { payload_idx },
            &reveal_tx,
            envelope.fee_rate,
            envelope.reveal_fee,
        )
        .await?;
        put_tx_entry_if_missing(
            broadcast_handle,
            rid,
            &reveal_tx,
            envelope.reveal_fee,
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
/// `signing_mode` is the mode resolved for the current canonical state, which the replacement's
/// tapscript has to still match; see the key check below.
///
/// Returns `Ok(None)` when no pending replacement exists for this payload, or when the one that
/// does can no longer be completed.
pub(crate) async fn complete_pending_reveal_replacement(
    payload_idx: u64,
    signature: &[u8; 64],
    signing_mode: EnvelopeSigningMode,
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

    // The replacement reuses the original tapscript, so its witness only validates under the key
    // that script commits to. If the canonical predicate rotated while the attempt was waiting, the
    // signer signed the replacement sighash under the new key and the RPC accepted it, because it
    // verifies against whatever key is current. Attaching it here would supersede a valid reveal
    // with one whose witness can never satisfy the script. Refuse and stop bumping this reveal,
    // matching what the fee bumper does when it notices the rotation before initiating one: the
    // original stays live at its current fee and a rebuild under the new key clears the error.
    let reveal_pubkey =
        reveal_script_pubkey(&reveal_script).map_err(|e| EnvelopeError::Other(e.into()))?;
    if !matches!(signing_mode, EnvelopeSigningMode::External { pubkey } if pubkey == reveal_pubkey)
    {
        warn!(
            payload_idx,
            %reveal_pubkey,
            ?signing_mode,
            "envelope signing key rotated since the reveal replacement was built; discarding it"
        );
        record.discard_pending_signature_replacement();
        record.set_terminal_error(TerminalError::UnsupportedRbfKind);
        broadcast_handle
            .put_tx_node(record)
            .await
            .map_err(|e| EnvelopeError::Other(e.into()))?;
        return Ok(None);
    }

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

    if previous_active_entry.is_none() {
        return Err(EnvelopeError::Other(anyhow::anyhow!(
            "previous reveal tx entry missing for pending replacement"
        )));
    }

    // One transaction: the replacement is inserted and the original superseded together, so there
    // is never a broadcastable replacement with nothing linking it to what it replaces.
    if !broadcast_handle
        .put_replacement_tx_entry(
            to_raw_buf32(previous_active_txid),
            to_raw_buf32(txid),
            L1TxEntry::from_tx_with_fee(&signed_tx, fee_rate, fee_sats),
        )
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?
    {
        // One reason the swap is refused is that it already happened: a previous run committed it
        // and stopped before the tx-node record caught up. Re-signing lands on the same txid,
        // because a taproot witness does not change one, so the retry is asking for a row that is
        // already there. Finish the activation instead of discarding a replacement the broadcaster
        // is publishing.
        if !replacement_swap_already_applied(broadcast_handle, previous_active_txid, txid).await? {
            // Otherwise the original has since gone invalid or confirmed: only `Unpublished` and
            // `Published` entries are replaceable. The pending attempt was built to supersede that
            // exact txid and can never be broadcast now; leaving it durable keeps the payload
            // pinned in `PendingRevealTxSign`, because the watcher waits on a replacement that
            // will never land and the fee bumper skips every node that carries one. Discard it so
            // both recover.
            warn!(
                ?previous_active_txid,
                "original reveal left the publishable state before the replacement could supersede it"
            );
            record.discard_pending_signature_replacement();
            broadcast_handle
                .put_tx_node(record)
                .await
                .map_err(|e| EnvelopeError::Other(e.into()))?;
            return Ok(None);
        }

        info!(
            ?txid,
            ?previous_active_txid,
            "adopting a reveal replacement a previous run already installed"
        );
    }

    // The durable swap is done, so the record must follow it. Activation cannot fail here: the
    // pending attempt was read above and nothing has removed it since.
    if !record.activate_pending_signature(attempt_parts(&signed_tx, fee_rate, fee_sats)) {
        return Err(EnvelopeError::Other(anyhow::anyhow!(
            "pending reveal replacement vanished while it was being activated"
        )));
    }
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

/// Reports whether the broadcast DB already carries the swap that was just refused.
///
/// The swap is one transaction, so an original recorded as replaced by exactly this txid, with the
/// replacement row present, can only come from a run that committed it and stopped before the
/// tx-node record was written. Anything else — a confirmation, an invalidation, a different
/// replacement — leaves the original naming something other than this txid.
async fn replacement_swap_already_applied(
    broadcast_handle: &L1BroadcastHandle,
    original_txid: L1TxId,
    replacement_txid: L1TxId,
) -> Result<bool, EnvelopeError> {
    let Some(original) = broadcast_handle
        .get_tx_entry_by_id_async(to_raw_buf32(original_txid))
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?
    else {
        return Ok(false);
    };
    if !matches!(original.status, L1TxStatus::Replaced { by } if by == replacement_txid) {
        return Ok(false);
    }

    Ok(broadcast_handle
        .get_tx_entry_by_id_async(to_raw_buf32(replacement_txid))
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?
        .is_some())
}

async fn put_tx_entry_if_missing(
    broadcast_handle: &L1BroadcastHandle,
    txid: L1TxId,
    tx: &Transaction,
    fee: Amount,
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
        .put_tx_entry(
            to_raw_buf32(txid),
            L1TxEntry::from_tx_with_fee(tx, envelope.fee_rate, fee),
        )
        .await
        .map_err(|e| EnvelopeError::Other(e.into()))?;
    Ok(())
}

async fn put_tx_node(
    broadcast_handle: &L1BroadcastHandle,
    kind: TxNodeKind,
    tx: &Transaction,
    fee_rate: FeeRate,
    fee_sats: Amount,
) -> Result<(), EnvelopeError> {
    let node_id = TxNodeId::from_kind(&kind);
    let attempt = TxAttempt::active(attempt_parts(tx, fee_rate, fee_sats), 0);
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
    use strata_csm_types::L1Payload;
    use strata_db_types::{
        fee_bump::{TerminalError, TxNodeId, TxNodeKind},
        l1_writer::{BundledPayloadEntry, L1BundleStatus},
    };
    use strata_l1_txfmt::TagData;
    use strata_primitives::buf::Buf32;

    use super::*;
    use crate::{
        test_utils::{
            test_context::{
                get_fee_bumping_writer_context, get_writer_context, get_writer_context_with_client,
            },
            TestBitcoinClient,
        },
        writer::{
            replacement::build::build_pending_single_reveal_replacement,
            test_utils::{get_broadcast_handle, get_envelope_ops},
            EnvelopeSigningMode, WriterContext,
        },
    };

    /// Extracts the external signing pubkey a test writer context is configured with.
    fn external_signing_pubkey<R: Reader + Signer + Wallet>(
        ctx: &Arc<WriterContext<R>>,
    ) -> XOnlyPublicKey {
        let EnvelopeSigningMode::External { pubkey } = ctx
            .signing_mode()
            .expect("test: writer context resolves a signing mode")
        else {
            panic!("test: writer context must use external signing");
        };
        pubkey
    }

    fn unsigned_test_entry() -> BundledPayloadEntry {
        let tag = TagData::new(1, 1, vec![]).unwrap();
        let payload = L1Payload::new(vec![vec![1; 150]; 1], tag).unwrap();
        BundledPayloadEntry::new_unsigned(payload)
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

        let EnvelopeSigningMode::External { pubkey } = ctx.signing_mode().unwrap() else {
            panic!("test writer context must use external signing");
        };
        let envelope = create_payload_envelopes(0, &entry, ctx, pubkey)
            .await
            .unwrap();

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

        let EnvelopeSigningMode::External { pubkey } = ctx.signing_mode().unwrap() else {
            panic!("test writer context must use external signing");
        };
        let err = create_payload_envelopes(0, &entry, ctx, pubkey)
            .await
            .unwrap_err();

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

        let entry = unsigned_test_entry();

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
        let pubkey = external_signing_pubkey(&ctx);
        let envelope = create_payload_envelopes(7, &entry, ctx, pubkey)
            .await
            .unwrap();
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

    /// Regression: the swap and the tx-node write are separate, so a run can commit the swap and
    /// stop before the record catches up. The retry re-signs to the same txid — a taproot witness
    /// does not change one — so the swap is refused as already done. Discarding the attempt there
    /// would abandon a replacement the broadcaster is already publishing and leave the record
    /// active on a superseded txid.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_complete_pending_reveal_replacement_adopts_an_already_committed_swap() {
        let bcast_handle = get_broadcast_handle();
        let ctx = get_fee_bumping_writer_context();
        let pubkey = external_signing_pubkey(&ctx);
        let envelope = create_payload_envelopes(7, &unsigned_test_entry(), ctx, pubkey)
            .await
            .unwrap();
        let signature = [1u8; 64];
        let original_reveal_txid =
            complete_reveal_and_broadcast(7, &envelope, &signature, &bcast_handle)
                .await
                .unwrap();
        let original_reveal_tx = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid))
            .await
            .unwrap()
            .expect("test: original reveal entry exists")
            .try_to_tx()
            .unwrap();

        let node_id = TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 });
        let mut record = bcast_handle
            .get_tx_node(node_id)
            .await
            .unwrap()
            .expect("test: reveal tx-node exists");
        let (pending, _) = build_pending_single_reveal_replacement(
            &original_reveal_tx,
            &envelope.commit_tx.output[0],
            FeeRate::from_sat_per_vb(envelope.fee_rate.to_sat_per_vb_ceil() + 5).unwrap(),
            1,
        )
        .unwrap();
        let replacement_txid = pending.txid;
        let replacement_entry = L1TxEntry::from_tx_with_fee(
            &pending.try_to_tx().unwrap(),
            pending.fee_rate().unwrap(),
            pending.fee(),
        );
        record.append_pending_signature_replacement(pending);
        bcast_handle.put_tx_node(record).await.unwrap();

        // What the interrupted run left behind: the swap committed, the record untouched.
        assert!(bcast_handle
            .put_replacement_tx_entry(
                to_raw_buf32(original_reveal_txid),
                to_raw_buf32(replacement_txid),
                replacement_entry,
            )
            .await
            .unwrap());

        let completed = complete_pending_reveal_replacement(
            7,
            &signature,
            EnvelopeSigningMode::External { pubkey },
            &bcast_handle,
        )
        .await
        .unwrap();

        assert_eq!(completed, Some(replacement_txid));
        let record = bcast_handle
            .get_tx_node(node_id)
            .await
            .unwrap()
            .expect("test: reveal tx-node exists");
        assert_eq!(record.active_txid, replacement_txid);
        assert_eq!(
            record.active_attempt().map(|attempt| attempt.status),
            Some(TxAttemptStatus::Active)
        );
        assert_eq!(record.pending_signature_attempt(), None);
    }

    /// Regression: a corrupt persisted pending transaction can decode with no inputs. Completing
    /// it must return an error instead of indexing the absent reveal input and crashing the writer
    /// task.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_complete_pending_reveal_replacement_rejects_a_zero_input_transaction() {
        let bcast_handle = get_broadcast_handle();
        let ctx = get_fee_bumping_writer_context();
        let pubkey = external_signing_pubkey(&ctx);
        let envelope = create_payload_envelopes(7, &unsigned_test_entry(), ctx, pubkey)
            .await
            .unwrap();
        let signature = [1u8; 64];
        let original_reveal_txid =
            complete_reveal_and_broadcast(7, &envelope, &signature, &bcast_handle)
                .await
                .unwrap();
        let original_reveal_tx = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid))
            .await
            .unwrap()
            .expect("test: original reveal entry exists")
            .try_to_tx()
            .unwrap();

        let node_id = TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx: 7 });
        let mut record = bcast_handle
            .get_tx_node(node_id)
            .await
            .unwrap()
            .expect("test: reveal tx-node exists");
        let (mut pending, _) = build_pending_single_reveal_replacement(
            &original_reveal_tx,
            &envelope.commit_tx.output[0],
            FeeRate::from_sat_per_vb(envelope.fee_rate.to_sat_per_vb_ceil() + 5).unwrap(),
            1,
        )
        .unwrap();
        // Segwit marker and flag followed by empty input and output vectors. The Bitcoin decoder
        // accepts this shape even though it cannot be a valid reveal transaction.
        pending.raw_tx = vec![2, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        assert!(pending.try_to_tx().unwrap().input.is_empty());
        record.append_pending_signature_replacement(pending);
        bcast_handle.put_tx_node(record).await.unwrap();

        let error = complete_pending_reveal_replacement(
            7,
            &signature,
            EnvelopeSigningMode::External { pubkey },
            &bcast_handle,
        )
        .await
        .expect_err("a zero-input pending reveal must be rejected");

        assert!(error
            .to_string()
            .contains("active reveal transaction is missing its tapscript witness"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_put_tx_node_refreshes_stale_terminal_record() {
        let bcast_handle = get_broadcast_handle();
        let ctx = get_fee_bumping_writer_context();
        let entry = unsigned_test_entry();
        let pubkey = external_signing_pubkey(&ctx);
        let envelope = create_payload_envelopes(7, &entry, ctx, pubkey)
            .await
            .unwrap();
        let kind = TxNodeKind::SingleEnvelopeCommit { payload_idx: 7 };

        put_tx_node(
            &bcast_handle,
            kind.clone(),
            &envelope.commit_tx,
            envelope.fee_rate,
            envelope.commit_fee,
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
        put_tx_node(
            &bcast_handle,
            kind,
            &replacement_tx,
            envelope.fee_rate,
            envelope.commit_fee,
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
