//! Watcher service for the btcio L1 writer.
//!
//! Drives the [`L1BundleStatus`] state machine for the current payload entry
//! on each timer tick.

use std::{
    collections::HashMap,
    future::Future,
    marker::PhantomData,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use bitcoin::secp256k1::XOnlyPublicKey;
use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use serde::Serialize;
use strata_btc_types::{Buf32BitcoinExt, TxidExt};
use strata_db_types::{
    common::L1TxId,
    fee_bump::{TxNodeId, TxNodeKind},
    l1_broadcast::{L1TxEntry, L1TxStatus},
    l1_writer::{BundledPayloadEntry, L1BundleStatus},
};
use strata_primitives::buf::Buf32;
use strata_service::{AsyncService, Response, Service, ServiceState};
use strata_status::StatusChannel;
use strata_storage::ops::writer::EnvelopeDataOps;
use tracing::*;

use crate::{
    broadcaster::L1BroadcastHandle,
    rpc_error::{is_retryable_envelope_error, retryable_reason},
    status::{apply_status_updates, L1StatusUpdate},
    writer::{
        builder::{EnvelopeData, EnvelopeError},
        context::{EnvelopeSigningMode, WriterContext},
        replacement::driver::{run_replacement_pass, ReplacementContext, ReplacementPacer},
        signer::{
            complete_pending_reveal_replacement, complete_reveal_and_broadcast,
            create_payload_envelopes, sign_and_broadcast_payload_envelopes,
        },
    },
};

fn to_l1_txid(txid: bitcoin::Txid) -> L1TxId {
    L1TxId::from(txid.to_buf32().0)
}

fn to_raw_buf32(txid: L1TxId) -> Buf32 {
    Buf32(txid.0)
}

/// Discards the pending externally-signed replacement on a payload's reveal tx-node.
async fn discard_pending_reveal_replacement(
    broadcast_handle: &L1BroadcastHandle,
    payload_idx: u64,
) -> anyhow::Result<()> {
    let node_id = TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx });
    let Some(mut record) = broadcast_handle.get_tx_node(node_id).await? else {
        return Ok(());
    };
    if record.discard_pending_signature_replacement() {
        broadcast_handle.put_tx_node(record).await?;
    }
    Ok(())
}

/// Abstracts the external dependencies of the watcher so that `process_input` can be
/// tested without a real Bitcoin node, database, or broadcast infrastructure.
pub(crate) trait WatcherServiceContext: Send + Sync + 'static {
    fn get_payload_entry(
        &self,
        idx: u64,
    ) -> impl Future<Output = anyhow::Result<Option<BundledPayloadEntry>>> + Send;

    fn put_payload_entry(
        &self,
        idx: u64,
        entry: BundledPayloadEntry,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Returns the current envelope signing mode.
    fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode>;

    fn create_envelopes(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
        envelope_pubkey: XOnlyPublicKey,
    ) -> impl Future<Output = Result<EnvelopeData, EnvelopeError>> + Send;

    fn sign_and_broadcast(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
    ) -> impl Future<Output = Result<(), EnvelopeError>> + Send;
    fn complete_reveal_and_broadcast(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
        envelope: &EnvelopeData,
        sig: &[u8; 64],
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Attaches an external signature to this payload's pending fee-bump reveal replacement.
    ///
    /// `signing_mode` is the mode resolved for the current canonical state; the replacement is
    /// refused when it no longer matches the key its tapscript commits to.
    fn complete_pending_reveal_replacement(
        &self,
        idx: u64,
        sig: &[u8; 64],
        signing_mode: EnvelopeSigningMode,
    ) -> impl Future<Output = anyhow::Result<Option<L1TxId>>> + Send;

    /// Reports whether a fee bump is waiting on a signature for this payload's reveal.
    fn has_pending_reveal_replacement(
        &self,
        idx: u64,
    ) -> impl Future<Output = anyhow::Result<bool>> + Send;

    /// Discards this payload's pending fee-bump reveal replacement, if it still has one.
    fn discard_pending_reveal_replacement(
        &self,
        idx: u64,
    ) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Returns the reveal txid this payload's tx-node currently considers active, if recorded.
    fn active_reveal_txid(
        &self,
        idx: u64,
    ) -> impl Future<Output = anyhow::Result<Option<L1TxId>>> + Send;

    fn get_tx_status(
        &self,
        txid: L1TxId,
    ) -> impl Future<Output = anyhow::Result<Option<(L1TxId, L1TxEntry)>>> + Send;

    fn report_status(
        &self,
        entry: &BundledPayloadEntry,
        status: &L1BundleStatus,
    ) -> impl Future<Output = ()> + Send;

    fn report_rpc_error(&self, reason: String) -> impl Future<Output = ()> + Send;

    /// Runs one RBF replacement pass over this writer's tx-node records.
    ///
    /// Driven from the watcher tick rather than a service of its own: rebuilding a reveal needs
    /// the writer's payload storage and signing mode, both of which live here.
    fn run_replacement_pass(&self) -> impl Future<Output = anyhow::Result<()>> + Send;
}

pub(crate) struct WatcherContextImpl<R: Reader + Signer + Wallet + Send + Sync + 'static> {
    context: Arc<WriterContext<R>>,
    ops: Arc<EnvelopeDataOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
    /// Held here rather than rebuilt per call so the interval actually spans watcher ticks.
    replacement_pacer: Arc<ReplacementPacer>,
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> WatcherContextImpl<R> {
    pub(crate) fn new(
        context: Arc<WriterContext<R>>,
        ops: Arc<EnvelopeDataOps>,
        broadcast_handle: Arc<L1BroadcastHandle>,
    ) -> Self {
        let replacement_pacer = Arc::new(ReplacementPacer::new(
            context.config.fee_bumping.check_interval(),
        ));
        Self {
            context,
            ops,
            broadcast_handle,
            replacement_pacer,
        }
    }
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> WatcherServiceContext
    for WatcherContextImpl<R>
{
    async fn run_replacement_pass(&self) -> anyhow::Result<()> {
        let replacement_context = ReplacementContext {
            envelope_ops: Some(self.ops.clone()),
            signing_mode_provider: Some(self.context.signing_mode_provider()),
            pacer: self.replacement_pacer.clone(),
            ..ReplacementContext::default()
        };
        run_replacement_pass(
            self.context.client.as_ref(),
            &self.context.config,
            self.broadcast_handle.as_ref(),
            &replacement_context,
        )
        .await
    }

    async fn get_payload_entry(&self, idx: u64) -> anyhow::Result<Option<BundledPayloadEntry>> {
        self.ops
            .get_payload_entry_by_idx_async(idx)
            .await
            .map_err(Into::into)
    }

    async fn put_payload_entry(&self, idx: u64, entry: BundledPayloadEntry) -> anyhow::Result<()> {
        self.ops
            .put_payload_entry_async(idx, entry)
            .await
            .map_err(Into::into)
    }

    fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode> {
        self.context.signing_mode()
    }

    async fn create_envelopes(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
        envelope_pubkey: XOnlyPublicKey,
    ) -> Result<EnvelopeData, EnvelopeError> {
        create_payload_envelopes(idx, entry, self.context.clone(), envelope_pubkey).await
    }

    async fn sign_and_broadcast(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
    ) -> Result<(), EnvelopeError> {
        sign_and_broadcast_payload_envelopes(
            idx,
            entry,
            self.context.clone(),
            self.ops.as_ref(),
            &self.broadcast_handle,
        )
        .await
    }

    async fn complete_reveal_and_broadcast(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
        envelope: &EnvelopeData,
        sig: &[u8; 64],
    ) -> anyhow::Result<()> {
        complete_reveal_and_broadcast(
            idx,
            entry,
            envelope,
            sig,
            self.ops.as_ref(),
            &self.broadcast_handle,
        )
        .await
        .map_err(Into::into)
    }

    async fn complete_pending_reveal_replacement(
        &self,
        idx: u64,
        sig: &[u8; 64],
        signing_mode: EnvelopeSigningMode,
    ) -> anyhow::Result<Option<L1TxId>> {
        complete_pending_reveal_replacement(idx, sig, signing_mode, &self.broadcast_handle)
            .await
            .map_err(Into::into)
    }

    async fn has_pending_reveal_replacement(&self, idx: u64) -> anyhow::Result<bool> {
        let node_id = TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx: idx });
        Ok(self
            .broadcast_handle
            .get_tx_node(node_id)
            .await?
            .is_some_and(|record| record.pending_signature_attempt().is_some()))
    }

    async fn discard_pending_reveal_replacement(&self, idx: u64) -> anyhow::Result<()> {
        discard_pending_reveal_replacement(&self.broadcast_handle, idx).await
    }

    async fn active_reveal_txid(&self, idx: u64) -> anyhow::Result<Option<L1TxId>> {
        let node_id = TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx: idx });
        Ok(self
            .broadcast_handle
            .get_tx_node(node_id)
            .await?
            .map(|record| record.active_txid))
    }

    async fn get_tx_status(&self, txid: L1TxId) -> anyhow::Result<Option<(L1TxId, L1TxEntry)>> {
        self.broadcast_handle
            .get_active_tx_entry_by_id_async(to_raw_buf32(txid))
            .await
            .map(|entry| entry.map(|(txid, entry)| (L1TxId::from(txid.0), entry)))
            .map_err(Into::into)
    }

    async fn report_status(&self, entry: &BundledPayloadEntry, status: &L1BundleStatus) {
        update_l1_status(entry, status, &self.context.status_channel).await;
    }

    async fn report_rpc_error(&self, reason: String) {
        let status_updates = [
            L1StatusUpdate::RpcConnected(false),
            L1StatusUpdate::RpcError(reason),
            L1StatusUpdate::LastUpdate(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            ),
        ];
        apply_status_updates(&status_updates, &self.context.status_channel).await;
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct WatcherStatus {
    pub(crate) current_payload_idx: u64,
    pub(crate) cache_size: usize,
}

pub(crate) struct WatcherState<C: WatcherServiceContext> {
    pub(crate) ctx: C,
    pub(crate) envelope_cache: HashMap<u64, EnvelopeData>,
    pub(crate) curr_payloadidx: u64,
}

impl<C: WatcherServiceContext> WatcherState<C> {
    pub(crate) fn new(ctx: C, curr_payloadidx: u64) -> Self {
        Self {
            ctx,
            envelope_cache: HashMap::new(),
            curr_payloadidx,
        }
    }
}

impl<C: WatcherServiceContext> ServiceState for WatcherState<C> {
    fn name(&self) -> &str {
        "btcio_watcher"
    }
}

pub(crate) struct WatcherService<C>(PhantomData<C>);

impl<C: WatcherServiceContext> Service for WatcherService<C> {
    type State = WatcherState<C>;
    type Msg = ();
    type Status = WatcherStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        WatcherStatus {
            current_payload_idx: state.curr_payloadidx,
            cache_size: state.envelope_cache.len(),
        }
    }
}

impl<C: WatcherServiceContext> AsyncService for WatcherService<C> {
    async fn process_input(state: &mut Self::State, _: Self::Msg) -> anyhow::Result<Response> {
        let dspan = debug_span!("process payload", idx=%state.curr_payloadidx);
        let _ = dspan.enter();

        // A failed pass must not stall payload processing; the next tick retries it.
        if let Err(error) = state.ctx.run_replacement_pass().await {
            warn!(%error, "payload envelope replacement pass failed");
        }

        if let Some(payloadentry) = state.ctx.get_payload_entry(state.curr_payloadidx).await? {
            match payloadentry.status {
                // If unsigned or needs resign, build envelope txs, sign commit with
                // wallet, and transition to PendingRevealTxSign awaiting the external
                // signer's Schnorr signature on the reveal tx.
                L1BundleStatus::Unsigned | L1BundleStatus::NeedsResign => {
                    state.handle_unsigned_or_needs_resign(payloadentry).await?;
                }

                // Waiting for the external signer to provide the reveal signature.
                // When the signature arrives (via RPC), complete the reveal tx and
                // transition to Unpublished.
                L1BundleStatus::PendingRevealTxSign(_) => {
                    state.handle_pending_reveal_tx_sign(payloadentry).await?;
                }

                // If finalized, nothing to do, move on to process next entry
                L1BundleStatus::Finalized | L1BundleStatus::Abandoned => {
                    state.curr_payloadidx += 1;
                }

                // If entry is signed but not finalized or excluded yet, check broadcast txs status
                L1BundleStatus::Published
                | L1BundleStatus::Confirmed
                | L1BundleStatus::Unpublished => {
                    state.handle_broadcast_status(payloadentry).await?;
                }
            }
        } else {
            // No payload exists, just continue the loop to wait for payload's presence in db
            debug!("Waiting for payloadentry to be present in db");
        }

        Ok(Response::Continue)
    }
}

impl<C: WatcherServiceContext> WatcherState<C> {
    /// Resolves the current envelope signing mode, deferring on failure.
    ///
    /// The signing mode is derived from dynamic ASM state, so a transient
    /// failure (e.g. a `DbError`) or a currently non-signable predicate (e.g.
    /// `NeverAccept`/`Sp1Groth16` after a rotation) must not be fatal: the
    /// writer service treats a `process_input` error as terminal. Returns
    /// `None` after logging so callers can defer and let the tick loop
    /// re-evaluate on the next pass instead of permanently killing the writer.
    fn resolve_signing_mode(&self) -> Option<EnvelopeSigningMode> {
        match self.ctx.signing_mode() {
            Ok(mode) => Some(mode),
            Err(err) => {
                warn!(%err, "could not resolve envelope signing mode; deferring to next tick");
                None
            }
        }
    }

    /// Builds envelope txs and transitions to `PendingRevealTxSign` or `Unpublished`.
    ///
    /// When an external signer is configured, signs the commit tx
    /// via wallet, caches the envelope, and waits for the reveal signature via RPC.
    /// When no external signer is needed, signs both in-process
    /// and transitions directly to `Unpublished`.
    async fn handle_unsigned_or_needs_resign(
        &mut self,
        payloadentry: BundledPayloadEntry,
    ) -> anyhow::Result<()> {
        debug!(current_status=?payloadentry.status);

        let Some(signing_mode) = self.resolve_signing_mode() else {
            return Ok(());
        };
        match signing_mode {
            EnvelopeSigningMode::External { pubkey } => match self
                .ctx
                .create_envelopes(self.curr_payloadidx, &payloadentry, pubkey)
                .await
            {
                Ok(envelope) => {
                    let cid = to_l1_txid(envelope.commit_tx.compute_txid());
                    let rid = to_l1_txid(envelope.reveal_tx.compute_txid());
                    let sighash = envelope.sighash;

                    let mut updated_entry = payloadentry.clone();
                    updated_entry.commit_txid = cid;
                    updated_entry.reveal_txid = rid;
                    updated_entry.payload_signature = None;
                    updated_entry.status = L1BundleStatus::PendingRevealTxSign(sighash);
                    self.ctx
                        .put_payload_entry(self.curr_payloadidx, updated_entry)
                        .await?;

                    self.envelope_cache.insert(self.curr_payloadidx, envelope);

                    debug!(%sighash, "envelope built, awaiting signer");
                }
                Err(EnvelopeError::NotEnoughUtxos(required, available)) => {
                    warn!(%required, %available, "waiting for sufficient utxos to create commit/reveal transaction");
                }
                Err(err) if err.is_blocked_by_fee_guardrail() => {
                    warn!(%err, "waiting for a transaction fee rate within the broadcast guardrail");
                }
                Err(err) if is_retryable_envelope_error(&err) => {
                    let reason = retryable_reason(&err);
                    warn!(%reason, "retrying envelope creation after Bitcoin RPC error");
                    self.ctx.report_rpc_error(reason).await;
                }
                Err(err) => {
                    return Err(err.into());
                }
            },
            EnvelopeSigningMode::InProcess => match self
                .ctx
                .sign_and_broadcast(self.curr_payloadidx, &payloadentry)
                .await
            {
                Ok(()) => debug!("envelope signed and queued for broadcast"),
                Err(EnvelopeError::NotEnoughUtxos(required, available)) => {
                    warn!(%required, %available, "waiting for sufficient utxos to create commit/reveal transaction");
                }
                Err(err) if err.is_blocked_by_fee_guardrail() => {
                    warn!(%err, "waiting for a transaction fee rate within the broadcast guardrail");
                }
                Err(err) if is_retryable_envelope_error(&err) => {
                    let reason = retryable_reason(&err);
                    warn!(%reason, "retrying envelope signing after Bitcoin RPC error");
                    self.ctx.report_rpc_error(reason).await;
                }
                Err(err) => {
                    return Err(err.into());
                }
            },
        }

        Ok(())
    }

    /// Completes the reveal tx and broadcasts both txs once the external sig arrives.
    ///
    /// On cache miss (e.g. restart), resets to `Unsigned` — safe because nothing
    /// has been broadcast yet.
    async fn handle_pending_reveal_tx_sign(
        &mut self,
        payloadentry: BundledPayloadEntry,
    ) -> anyhow::Result<()> {
        let Some(sig) = &payloadentry.payload_signature else {
            // A fee-bump replacement can be waiting on a signature that will never arrive because
            // the original reveal confirmed first. Advance off the pending state instead of
            // blocking on the signer forever.
            if self
                .advance_if_reveal_replacement_already_confirmed(&payloadentry)
                .await?
            {
                return Ok(());
            }

            // Resetting to `Unsigned` rebuilds the envelope from fresh UTXOs, which is only safe
            // while nothing has been broadcast. A fee bump breaks that assumption: it moves an
            // already-published reveal back into `PendingRevealTxSign` while the original stays
            // live, so rebuilding here would publish the same payload twice.
            if self
                .ctx
                .has_pending_reveal_replacement(self.curr_payloadidx)
                .await?
            {
                // Unless what it supersedes has itself gone invalid, in which case waiting pins
                // the payload here for good: the replacement spends inputs that are gone so no
                // signature can rescue it, and the fee bumper skips every node carrying a pending
                // attempt.
                if self
                    .abandon_reveal_replacement_if_invalid(&payloadentry)
                    .await?
                {
                    return Ok(());
                }
                trace!("waiting for signer to sign the fee-bump reveal replacement");
                return Ok(());
            }

            let Some(envelope) = self.envelope_cache.get(&self.curr_payloadidx) else {
                // Cache miss (e.g. restart) — reset to Unsigned to rebuild
                // envelope from scratch (new UTXOs, new sighash).
                // Safe: nothing has been broadcast yet.
                warn!("envelope not in cache, resetting to Unsigned");
                let mut updated_entry = payloadentry.clone();
                updated_entry.payload_signature = None;
                updated_entry.status = L1BundleStatus::Unsigned;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;
                return Ok(());
            };

            let Some(signing_mode) = self.resolve_signing_mode() else {
                return Ok(());
            };
            match signing_mode {
                EnvelopeSigningMode::External { pubkey } if pubkey == envelope.envelope_pubkey => {}
                _ => {
                    warn!("envelope signing mode changed, resetting to Unsigned");
                    let mut updated_entry = payloadentry.clone();
                    updated_entry.payload_signature = None;
                    updated_entry.status = L1BundleStatus::Unsigned;
                    self.ctx
                        .put_payload_entry(self.curr_payloadidx, updated_entry)
                        .await?;
                    self.envelope_cache.remove(&self.curr_payloadidx);
                    return Ok(());
                }
            }

            trace!("waiting for signer to provide reveal signature");
            return Ok(());
        };
        let envelope = match self.envelope_cache.remove(&self.curr_payloadidx) {
            Some(envelope) => envelope,
            None => {
                // Resolved before completion, not after: a fee-bump replacement reuses the original
                // reveal's tapscript, so an external signature is only usable while the canonical
                // key still matches what that script commits to. A resolution failure only skips
                // the completion attempt, which the pending attempt survives; the recovery paths
                // below still need to run.
                if let Some(signing_mode) = self.resolve_signing_mode() {
                    if let Some(rid) = self
                        .ctx
                        .complete_pending_reveal_replacement(
                            self.curr_payloadidx,
                            sig.as_ref(),
                            signing_mode,
                        )
                        .await?
                    {
                        let mut updated_entry = payloadentry.clone();
                        updated_entry.reveal_txid = rid;
                        updated_entry.status = L1BundleStatus::Unpublished;
                        self.ctx
                            .put_payload_entry(self.curr_payloadidx, updated_entry)
                            .await?;
                        debug!(reveal_txid = ?rid, "pending reveal replacement signed and stored for broadcast");
                        return Ok(());
                    }
                }
                if self
                    .advance_if_reveal_replacement_already_confirmed(&payloadentry)
                    .await?
                {
                    return Ok(());
                }

                if self
                    .ctx
                    .has_pending_reveal_replacement(self.curr_payloadidx)
                    .await?
                {
                    // The replacement could not be completed this tick but the original reveal
                    // is still live. Rebuilding would double-publish the payload.
                    warn!(
                        payload_idx = %self.curr_payloadidx,
                        "pending fee-bump reveal replacement could not be completed, retrying"
                    );
                    return Ok(());
                }

                // A crash between activating a replacement and writing the payload row leaves no
                // pending attempt, so completion above returned `None`. The activated replacement
                // is live, so reconcile the payload row from the tx-node instead of rebuilding.
                //
                // Only ever move forward: adopt the node's txid when the payload's current reveal
                // resolves *through the replacement chain* to it. The opposite partial write
                // (payload newer than node) must not drag the payload back to a superseded reveal.
                let node_reveal_txid = self.ctx.active_reveal_txid(self.curr_payloadidx).await?;
                let reconcile_to = match node_reveal_txid {
                    Some(node_txid) if node_txid != payloadentry.reveal_txid => self
                        .ctx
                        .get_tx_status(payloadentry.reveal_txid)
                        .await?
                        .and_then(|(resolved_txid, _)| {
                            (resolved_txid == node_txid).then_some(node_txid)
                        }),
                    _ => None,
                };

                if let Some(active_reveal_txid) = reconcile_to {
                    warn!(
                        payload_idx = %self.curr_payloadidx,
                        ?active_reveal_txid,
                        "reconciling payload row with an already-activated reveal replacement"
                    );
                    let mut updated_entry = payloadentry.clone();
                    updated_entry.reveal_txid = active_reveal_txid;
                    updated_entry.status = L1BundleStatus::Unpublished;
                    self.ctx
                        .put_payload_entry(self.curr_payloadidx, updated_entry)
                        .await?;
                    return Ok(());
                }

                // The payload's own transactions may still be live: a replacement that was refused
                // leaves the original published, and so does a stop between broadcasting and
                // writing the payload row. Resetting to `Unsigned` here rebuilds against fresh
                // UTXOs and would put the same payload on L1 twice, so adopt what the broadcaster
                // is tracking instead.
                if self
                    .reconcile_payload_with_broadcast_state(&payloadentry)
                    .await?
                {
                    return Ok(());
                }

                warn!(
                    payload_idx = %self.curr_payloadidx,
                    commit_txid = ?payloadentry.commit_txid,
                    reveal_txid = ?payloadentry.reveal_txid,
                    "envelope not in cache, resetting to Unsigned");
                let mut updated_entry = payloadentry.clone();
                updated_entry.payload_signature = None;
                updated_entry.status = L1BundleStatus::Unsigned;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;
                return Ok(());
            }
        };
        let Some(signing_mode) = self.resolve_signing_mode() else {
            // Preserve the cached envelope so a transient failure does not force
            // a needless rebuild + external re-sign on the next tick.
            self.envelope_cache.insert(self.curr_payloadidx, envelope);
            return Ok(());
        };
        match signing_mode {
            EnvelopeSigningMode::External { pubkey } if pubkey == envelope.envelope_pubkey => {}
            _ => {
                warn!("envelope signing mode changed, resetting to Unsigned");
                let mut updated_entry = payloadentry.clone();
                updated_entry.payload_signature = None;
                updated_entry.status = L1BundleStatus::Unsigned;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;
                return Ok(());
            }
        }
        // The signature the RPC stored was validated against the sighash on the payload row, which
        // is not necessarily the one this cached envelope commits to: the fee bumper writes that
        // row too, so a row write racing a rebuild can leave the two describing different reveals.
        // Attaching a signature over the wrong sighash produces a witness that can never validate,
        // and a reveal rejected on script grounds never becomes `InvalidInputs`, so nothing would
        // rebuild it. Rebuild now instead, while nothing has been broadcast.
        if !matches!(
            payloadentry.status,
            L1BundleStatus::PendingRevealTxSign(sighash) if sighash == envelope.sighash
        ) {
            warn!(
                payload_idx = %self.curr_payloadidx,
                envelope_sighash = %envelope.sighash,
                "signature does not cover the cached envelope; resetting to Unsigned"
            );
            let mut updated_entry = payloadentry.clone();
            updated_entry.payload_signature = None;
            updated_entry.status = L1BundleStatus::Unsigned;
            self.ctx
                .put_payload_entry(self.curr_payloadidx, updated_entry)
                .await?;
            return Ok(());
        }

        match self
            .ctx
            .complete_reveal_and_broadcast(
                self.curr_payloadidx,
                &payloadentry,
                &envelope,
                sig.as_ref(),
            )
            .await
        {
            Ok(()) => debug!("reveal signed and stored for broadcast"),
            Err(e) => {
                error!(%e, "failed to attach reveal signature");
            }
        }
        Ok(())
    }

    async fn advance_if_reveal_replacement_already_confirmed(
        &mut self,
        payloadentry: &BundledPayloadEntry,
    ) -> anyhow::Result<bool> {
        let Some((_, reveal_tx)) = self.ctx.get_tx_status(payloadentry.reveal_txid).await? else {
            return Ok(false);
        };
        if !matches!(
            reveal_tx.status,
            L1TxStatus::Confirmed { .. } | L1TxStatus::Finalized { .. }
        ) {
            return Ok(false);
        }
        if !self
            .reconcile_payload_with_broadcast_state(payloadentry)
            .await?
        {
            return Ok(false);
        }

        debug!(
            payload_idx = self.curr_payloadidx,
            "pending reveal replacement discarded because original reveal already advanced"
        );
        Ok(true)
    }

    /// Abandons a pending fee-bump reveal replacement once the payload's own transactions are dead.
    ///
    /// A pending attempt is built to supersede one exact txid. When that transaction, or the commit
    /// it spends, reaches [`L1TxStatus::InvalidInputs`], the replacement can never be broadcast, so
    /// there is nothing left to wait for and rebuilding is the only way forward — the same verdict
    /// [`Self::reconcile_payload_with_broadcast_state`] reaches for an invalid reveal outside a fee
    /// bump. Resetting is safe for the same reason: the inputs are gone, so nothing this payload
    /// published can still confirm.
    ///
    /// Returns whether the payload was reset for a rebuild.
    async fn abandon_reveal_replacement_if_invalid(
        &mut self,
        payloadentry: &BundledPayloadEntry,
    ) -> anyhow::Result<bool> {
        let commit_status = self
            .ctx
            .get_tx_status(payloadentry.commit_txid)
            .await?
            .map(|(_, entry)| entry.status);
        let reveal_status = self
            .ctx
            .get_tx_status(payloadentry.reveal_txid)
            .await?
            .map(|(_, entry)| entry.status);
        if !matches!(commit_status, Some(L1TxStatus::InvalidInputs))
            && !matches!(reveal_status, Some(L1TxStatus::InvalidInputs))
        {
            return Ok(false);
        }

        warn!(
            payload_idx = %self.curr_payloadidx,
            ?commit_status,
            ?reveal_status,
            "discarding a pending reveal replacement whose original went invalid"
        );
        self.ctx
            .discard_pending_reveal_replacement(self.curr_payloadidx)
            .await?;
        self.envelope_cache.remove(&self.curr_payloadidx);

        let mut updated_entry = payloadentry.clone();
        updated_entry.payload_signature = None;
        updated_entry.status = L1BundleStatus::Unsigned;
        self.ctx
            .put_payload_entry(self.curr_payloadidx, updated_entry)
            .await?;
        Ok(true)
    }

    /// Re-points the payload row at the transactions the broadcaster is already tracking.
    ///
    /// Both txids are resolved through the replacement chain, so a payload naming a superseded
    /// attempt lands on the one that is actually live. Returns whether the row was rewritten;
    /// `false` means there is nothing live to track and the caller is free to rebuild.
    ///
    /// A reveal with `InvalidInputs` counts as nothing live: its inputs are gone, so rebuilding is
    /// the only way forward.
    async fn reconcile_payload_with_broadcast_state(
        &mut self,
        payloadentry: &BundledPayloadEntry,
    ) -> anyhow::Result<bool> {
        let Some((commit_txid, commit_tx)) =
            self.ctx.get_tx_status(payloadentry.commit_txid).await?
        else {
            return Ok(false);
        };
        let Some((reveal_txid, reveal_tx)) =
            self.ctx.get_tx_status(payloadentry.reveal_txid).await?
        else {
            return Ok(false);
        };
        if matches!(reveal_tx.status, L1TxStatus::InvalidInputs) {
            return Ok(false);
        }

        let new_status = determine_payload_next_status(&commit_tx.status, &reveal_tx.status);
        let mut updated_entry = payloadentry.clone();
        updated_entry.commit_txid = commit_txid;
        updated_entry.reveal_txid = reveal_txid;
        updated_entry.payload_signature = None;
        updated_entry.status = new_status.clone();
        self.ctx.report_status(&updated_entry, &new_status).await;
        self.ctx
            .put_payload_entry(self.curr_payloadidx, updated_entry)
            .await?;

        if new_status == L1BundleStatus::Finalized {
            self.curr_payloadidx += 1;
        }

        debug!(
            payload_idx = self.curr_payloadidx,
            ?commit_txid,
            ?reveal_txid,
            ?new_status,
            "payload row reconciled with broadcast state"
        );
        Ok(true)
    }

    /// Checks broadcast tx statuses and advances the payload state machine.
    async fn handle_broadcast_status(
        &mut self,
        payloadentry: BundledPayloadEntry,
    ) -> anyhow::Result<()> {
        trace!("Checking payloadentry's broadcast status");
        let commit_tx = self.ctx.get_tx_status(payloadentry.commit_txid).await?;
        let reveal_tx = self.ctx.get_tx_status(payloadentry.reveal_txid).await?;

        match (commit_tx, reveal_tx) {
            (Some((commit_txid, ctx)), Some((reveal_txid, rtx))) => {
                let new_status = determine_payload_next_status(&ctx.status, &rtx.status);
                debug!(?new_status, "The next status for payload");
                if matches!(
                    new_status,
                    L1BundleStatus::Confirmed | L1BundleStatus::Finalized
                ) {
                    debug!(
                        component = "btcio_writer",
                        payload_idx = self.curr_payloadidx,
                        ?commit_txid,
                        ?reveal_txid,
                        payload_status = ?new_status,
                        commit_l1_status = ?ctx.status,
                        reveal_l1_status = ?rtx.status,
                        "payload advanced on L1"
                    );
                }

                let mut updated_entry = payloadentry.clone();
                updated_entry.commit_txid = commit_txid;
                updated_entry.reveal_txid = reveal_txid;
                updated_entry.status = new_status.clone();
                self.ctx.report_status(&updated_entry, &new_status).await;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;

                if new_status.is_terminal() {
                    self.curr_payloadidx += 1;
                }
            }
            _ => {
                warn!("Corresponding commit/reveal entry for payloadentry not found in broadcast db. Sign and create transactions again.");
                let mut updated_entry = payloadentry.clone();
                updated_entry.payload_signature = None;
                updated_entry.status = L1BundleStatus::Unsigned;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;
            }
        }
        Ok(())
    }
}

async fn update_l1_status(
    payloadentry: &BundledPayloadEntry,
    new_status: &L1BundleStatus,
    status_channel: &StatusChannel,
) {
    // Update L1 status. Since we are processing one payloadentry at a time, if the entry is
    // finalized/confirmed, then it means it is published as well
    if new_status.has_reached_l1() {
        let status_updates = [
            L1StatusUpdate::LastPublishedTxid(to_raw_buf32(payloadentry.reveal_txid).to_txid()),
            L1StatusUpdate::IncrementPublishedRevealCount,
        ];
        apply_status_updates(&status_updates, status_channel).await;
    }
}

/// Determine the status of the `PayloadEntry` based on the status of its commit and reveal
/// transactions in bitcoin.
pub(crate) fn determine_payload_next_status(
    commit_status: &L1TxStatus,
    reveal_status: &L1TxStatus,
) -> L1BundleStatus {
    match (&commit_status, &reveal_status) {
        // If reveal is finalized, both are finalized
        (_, L1TxStatus::Finalized { .. }) => L1BundleStatus::Finalized,
        // If reveal is confirmed, both are confirmed
        (_, L1TxStatus::Confirmed { .. }) => L1BundleStatus::Confirmed,
        // If reveal is published regardless of commit, the payload is published
        (_, L1TxStatus::Published) => L1BundleStatus::Published,
        // Replacement chains are normally followed before this function. If a
        // stale entry reaches here, keep the watcher from needlessly resigning.
        (L1TxStatus::Replaced { .. }, _) | (_, L1TxStatus::Replaced { .. }) => {
            L1BundleStatus::Published
        }
        // Invalidating the commit explicitly requests a rebuild.
        (L1TxStatus::InvalidInputs, _) => L1BundleStatus::NeedsResign,
        (L1TxStatus::Abandoned, _) | (_, L1TxStatus::Abandoned) => L1BundleStatus::Abandoned,
        // If commit is unpublished, both are upublished
        (L1TxStatus::Queued | L1TxStatus::Unpublished | L1TxStatus::Submitting, _) => {
            L1BundleStatus::Unpublished
        }
        // If commit is published but not reveal, the payload is unpublished
        (_, L1TxStatus::Queued | L1TxStatus::Unpublished | L1TxStatus::Submitting) => {
            L1BundleStatus::Unpublished
        }
        // If reveal has invalid inputs, these need resign because we can do nothing with just
        // commit tx confirmed. This should not occur in practice
        (_, L1TxStatus::InvalidInputs) => L1BundleStatus::NeedsResign,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use anyhow::anyhow;
    use bitcoin::{
        absolute::LockTime,
        blockdata::{opcodes, script::Builder as ScriptBuilder},
        hashes::Hash,
        key::UntweakedKeypair,
        secp256k1::{XOnlyPublicKey, SECP256K1},
        taproot::TaprootBuilder,
        transaction::Version,
        Amount, FeeRate, OutPoint, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
    };
    use bitcoind_async_client::error::ClientError;
    use strata_csm_types::L1Payload;
    use strata_db_types::{
        common::L1TxId,
        fee_bump::{TerminalError, TxAttemptStatus, TxNodeId, TxNodeKind, TxNodeRecord},
        l1_broadcast::L1TxEntry,
        l1_writer::{BundledPayloadEntry, L1BundleStatus},
    };
    use strata_l1_txfmt::TagData;
    use strata_primitives::buf::{Buf32, Buf64};

    use super::*;
    use crate::{
        broadcaster::L1BroadcastHandle,
        tx_entry::L1TxEntryExt,
        writer::{
            builder::{EnvelopeData, EnvelopeError},
            replacement::build::build_pending_single_reveal_replacement,
            signer::{complete_pending_reveal_replacement, complete_reveal_and_broadcast},
            test_utils::{get_broadcast_handle, get_envelope_ops},
        },
    };

    const TEST_REQUIRED_SATS: u64 = 4096;
    const TEST_AVAILABLE_SATS: u64 = 2658;

    #[derive(Clone, Copy)]
    enum MockEnvelopeFailure {
        NotEnoughUtxos,
        FeeGuardrail,
        PrereqFetch,
        SignRawTransaction,
        Other,
    }

    impl MockEnvelopeFailure {
        fn into_error(self) -> EnvelopeError {
            match self {
                Self::NotEnoughUtxos => {
                    EnvelopeError::NotEnoughUtxos(TEST_REQUIRED_SATS, TEST_AVAILABLE_SATS)
                }
                Self::FeeGuardrail => EnvelopeError::ResolvedFeeRateAboveMax {
                    resolved_sat_vb: 101,
                    ceiling_sat_vb: 100,
                },
                Self::PrereqFetch => EnvelopeError::PrereqFetch(anyhow::Error::from(
                    ClientError::Connection("mock connection failure".to_string()),
                )),
                Self::SignRawTransaction => EnvelopeError::SignRawTransaction(
                    ClientError::Connection("mock signing failure".to_string()),
                ),
                Self::Other => EnvelopeError::Other(anyhow!("mock storage failure")),
            }
        }
    }

    fn minimal_envelope_data() -> EnvelopeData {
        minimal_envelope_data_for(&[1u8; 32])
    }

    fn minimal_envelope_data_for(seckey: &[u8; 32]) -> EnvelopeData {
        let keypair = UntweakedKeypair::from_seckey_slice(SECP256K1, seckey).expect("valid key");
        let pubkey = XOnlyPublicKey::from_keypair(&keypair).0;
        // A single `<pubkey> OP_CHECKSIG` leaf, as SPS-51 reveal scripts open: the control_block
        // lookup in attach_reveal_signature succeeds, and the key the script commits to is the one
        // a replacement's signature has to verify against.
        let reveal_script = ScriptBuilder::new()
            .push_slice(pubkey.serialize())
            .push_opcode(opcodes::all::OP_CHECKSIG)
            .into_script();
        let taproot_spend_info = TaprootBuilder::new()
            .add_leaf(0, reveal_script.clone())
            .expect("valid leaf")
            .finalize(SECP256K1, pubkey)
            .expect("valid taproot");
        let dummy_input = TxIn {
            previous_output: OutPoint {
                txid: Txid::all_zeros(),
                vout: 0,
            },
            script_sig: ScriptBuf::new(),
            sequence: Sequence::MAX,
            witness: Witness::new(),
        };
        let commit_tx = Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![dummy_input.clone()],
            output: vec![TxOut {
                value: Amount::from_sat(1000),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        let reveal_tx = Transaction {
            version: Version(2),
            lock_time: LockTime::ZERO,
            input: vec![dummy_input],
            output: vec![TxOut {
                value: Amount::from_sat(546),
                script_pubkey: ScriptBuf::new(),
            }],
        };
        EnvelopeData::new(
            commit_tx,
            reveal_tx,
            Buf32([42u8; 32]),
            reveal_script,
            taproot_spend_info,
            pubkey,
            FeeRate::from_sat_per_vb(2).expect("test: valid fee rate"),
            Amount::from_sat(100),
            Amount::from_sat(50),
        )
    }

    struct MockWatcherContext {
        stored: Mutex<HashMap<u64, BundledPayloadEntry>>,
        broadcast_handle: Arc<L1BroadcastHandle>,
        signing_mode: Mutex<EnvelopeSigningMode>,
        signing_mode_fails: Mutex<bool>,
        create_failure: Option<MockEnvelopeFailure>,
        sign_failure: Option<MockEnvelopeFailure>,
        rpc_errors: Mutex<Vec<String>>,
    }

    impl MockWatcherContext {
        fn new(external_signing: bool) -> Self {
            let signing_mode = if external_signing {
                EnvelopeSigningMode::External {
                    pubkey: minimal_envelope_data().envelope_pubkey,
                }
            } else {
                EnvelopeSigningMode::InProcess
            };
            Self {
                stored: Mutex::new(HashMap::new()),
                broadcast_handle: get_broadcast_handle(),
                signing_mode: Mutex::new(signing_mode),
                signing_mode_fails: Mutex::new(false),
                create_failure: None,
                sign_failure: None,
                rpc_errors: Mutex::new(Vec::new()),
            }
        }

        fn with_create_not_enough_utxos(mut self) -> Self {
            self.create_failure = Some(MockEnvelopeFailure::NotEnoughUtxos);
            self
        }

        fn with_sign_not_enough_utxos(mut self) -> Self {
            self.sign_failure = Some(MockEnvelopeFailure::NotEnoughUtxos);
            self
        }

        fn with_create_fee_guardrail(mut self) -> Self {
            self.create_failure = Some(MockEnvelopeFailure::FeeGuardrail);
            self
        }

        fn with_sign_fee_guardrail(mut self) -> Self {
            self.sign_failure = Some(MockEnvelopeFailure::FeeGuardrail);
            self
        }

        fn with_create_prereq_fetch(mut self) -> Self {
            self.create_failure = Some(MockEnvelopeFailure::PrereqFetch);
            self
        }

        fn with_sign_raw_transaction_failure(mut self) -> Self {
            self.sign_failure = Some(MockEnvelopeFailure::SignRawTransaction);
            self
        }

        fn with_sign_other_failure(mut self) -> Self {
            self.sign_failure = Some(MockEnvelopeFailure::Other);
            self
        }

        fn get_stored(&self, idx: u64) -> Option<BundledPayloadEntry> {
            self.stored.lock().unwrap().get(&idx).cloned()
        }

        fn rpc_error_count(&self) -> usize {
            self.rpc_errors.lock().unwrap().len()
        }

        fn set_signing_mode(&self, signing_mode: EnvelopeSigningMode) {
            *self.signing_mode.lock().unwrap() = signing_mode;
        }

        fn set_signing_mode_failure(&self, fails: bool) {
            *self.signing_mode_fails.lock().unwrap() = fails;
        }
    }

    impl WatcherServiceContext for MockWatcherContext {
        // Replacement is exercised by the driver's own tests; the watcher tests care about
        // payload-status transitions, so this is a no-op here.
        async fn run_replacement_pass(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn get_payload_entry(&self, idx: u64) -> anyhow::Result<Option<BundledPayloadEntry>> {
            Ok(self.stored.lock().unwrap().get(&idx).cloned())
        }

        async fn put_payload_entry(
            &self,
            idx: u64,
            entry: BundledPayloadEntry,
        ) -> anyhow::Result<()> {
            self.stored.lock().unwrap().insert(idx, entry);
            Ok(())
        }

        fn signing_mode(&self) -> anyhow::Result<EnvelopeSigningMode> {
            if *self.signing_mode_fails.lock().unwrap() {
                anyhow::bail!("mock signing mode failure");
            }
            Ok(*self.signing_mode.lock().unwrap())
        }

        async fn create_envelopes(
            &self,
            _idx: u64,
            _entry: &BundledPayloadEntry,
            _envelope_pubkey: XOnlyPublicKey,
        ) -> Result<EnvelopeData, EnvelopeError> {
            if let Some(failure) = self.create_failure {
                return Err(failure.into_error());
            }
            Ok(minimal_envelope_data())
        }

        async fn sign_and_broadcast(
            &self,
            idx: u64,
            entry: &BundledPayloadEntry,
        ) -> Result<(), EnvelopeError> {
            if let Some(failure) = self.sign_failure {
                return Err(failure.into_error());
            }
            let mut linked = entry.clone();
            linked.commit_txid = L1TxId::from([1u8; 32]);
            linked.reveal_txid = L1TxId::from([2u8; 32]);
            linked.status = L1BundleStatus::Unpublished;
            self.stored.lock().unwrap().insert(idx, linked);
            Ok(())
        }

        async fn complete_reveal_and_broadcast(
            &self,
            idx: u64,
            entry: &BundledPayloadEntry,
            envelope: &EnvelopeData,
            sig: &[u8; 64],
        ) -> anyhow::Result<()> {
            let ops = get_envelope_ops();
            complete_reveal_and_broadcast(
                idx,
                entry,
                envelope,
                sig,
                ops.as_ref(),
                &self.broadcast_handle,
            )
            .await
            .map_err(anyhow::Error::from)?;
            let linked = ops
                .get_payload_entry_by_idx_async(idx)
                .await?
                .expect("successful persistence stores writer linkage");
            self.stored.lock().unwrap().insert(idx, linked);
            Ok(())
        }

        async fn complete_pending_reveal_replacement(
            &self,
            idx: u64,
            sig: &[u8; 64],
            signing_mode: EnvelopeSigningMode,
        ) -> anyhow::Result<Option<L1TxId>> {
            complete_pending_reveal_replacement(idx, sig, signing_mode, &self.broadcast_handle)
                .await
                .map_err(Into::into)
        }

        async fn has_pending_reveal_replacement(&self, idx: u64) -> anyhow::Result<bool> {
            let node_id =
                TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx: idx });
            Ok(self
                .broadcast_handle
                .get_tx_node(node_id)
                .await?
                .is_some_and(|record| record.pending_signature_attempt().is_some()))
        }

        async fn discard_pending_reveal_replacement(&self, idx: u64) -> anyhow::Result<()> {
            discard_pending_reveal_replacement(&self.broadcast_handle, idx).await
        }

        async fn active_reveal_txid(&self, idx: u64) -> anyhow::Result<Option<L1TxId>> {
            let node_id =
                TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx: idx });
            Ok(self
                .broadcast_handle
                .get_tx_node(node_id)
                .await?
                .map(|record| record.active_txid))
        }

        async fn get_tx_status(&self, txid: L1TxId) -> anyhow::Result<Option<(L1TxId, L1TxEntry)>> {
            self.broadcast_handle
                .get_active_tx_entry_by_id_async(to_raw_buf32(txid))
                .await
                .map(|entry| entry.map(|(txid, entry)| (L1TxId::from(txid.0), entry)))
                .map_err(Into::into)
        }

        async fn report_status(&self, _entry: &BundledPayloadEntry, _status: &L1BundleStatus) {}

        async fn report_rpc_error(&self, reason: String) {
            self.rpc_errors.lock().unwrap().push(reason);
        }
    }

    fn test_unsigned_entry() -> BundledPayloadEntry {
        let tag = TagData::new(1, 1, vec![]).unwrap();
        let payload = L1Payload::new(vec![vec![1; 150]; 1], tag).unwrap();
        BundledPayloadEntry::new_unsigned(payload)
    }

    #[tokio::test]
    async fn test_unchecked_transitions_to_unpublished() {
        let ctx = MockWatcherContext::new(false);
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unpublished);
        assert_eq!(stored.commit_txid, L1TxId::from([1u8; 32]));
        assert_eq!(stored.reveal_txid, L1TxId::from([2u8; 32]));
        // No cache entry — ephemeral path does not use the envelope cache
        assert!(state.envelope_cache.is_empty());
    }

    #[tokio::test]
    async fn test_unchecked_not_enough_utxos_keeps_unsigned_for_retry() {
        let ctx = MockWatcherContext::new(false).with_sign_not_enough_utxos();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        // Unsigned entries use zero txids as sentinels because no txs have been built yet.
        assert_eq!(stored.commit_txid, L1TxId::zero());
        assert_eq!(stored.reveal_txid, L1TxId::zero());
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
    }

    #[tokio::test]
    async fn test_unchecked_fee_guardrail_keeps_unsigned_for_rebuild() {
        let ctx = MockWatcherContext::new(false).with_sign_fee_guardrail();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        assert_eq!(
            state.ctx.get_stored(0).unwrap().status,
            L1BundleStatus::Unsigned
        );
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
        assert_eq!(state.ctx.rpc_error_count(), 0);
    }

    #[tokio::test]
    async fn test_broadcast_status_resolves_active_fee_bump_txids() {
        let ctx = MockWatcherContext::new(false);
        let mut entry = test_unsigned_entry();
        entry.status = L1BundleStatus::Published;
        entry.commit_txid = L1TxId::from([0x10; 32]);
        entry.reveal_txid = L1TxId::from([0x20; 32]);

        let replacement_commit_txid = L1TxId::from([0x11; 32]);
        let replacement_reveal_txid = L1TxId::from([0x21; 32]);
        let envelope = minimal_envelope_data();
        let finalized = L1TxStatus::Finalized {
            confirmations: 6,
            block_hash: Buf32::from([0xBB; 32]),
            block_height: 100,
        };
        let mut replacement_commit_entry = L1TxEntry::from_tx(&envelope.commit_tx);
        replacement_commit_entry.status = finalized.clone();
        let mut replacement_reveal_entry = L1TxEntry::from_tx(&envelope.reveal_tx);
        replacement_reveal_entry.status = finalized;

        let mut original_commit_entry = L1TxEntry::from_tx(&envelope.commit_tx);
        original_commit_entry.status = L1TxStatus::Replaced {
            by: replacement_commit_txid,
        };
        let mut original_reveal_entry = L1TxEntry::from_tx(&envelope.reveal_tx);
        original_reveal_entry.status = L1TxStatus::Replaced {
            by: replacement_reveal_txid,
        };

        for (txid, tx_entry) in [
            (to_raw_buf32(entry.commit_txid), original_commit_entry),
            (to_raw_buf32(entry.reveal_txid), original_reveal_entry),
            (
                to_raw_buf32(replacement_commit_txid),
                replacement_commit_entry,
            ),
            (
                to_raw_buf32(replacement_reveal_txid),
                replacement_reveal_entry,
            ),
        ] {
            ctx.broadcast_handle
                .put_tx_entry(txid, tx_entry)
                .await
                .unwrap();
        }

        let mut state = WatcherState::new(ctx, 0);
        state.handle_broadcast_status(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Finalized);
        assert_eq!(stored.commit_txid, replacement_commit_txid);
        assert_eq!(stored.reveal_txid, replacement_reveal_txid);
        assert_eq!(state.curr_payloadidx, 1);
    }

    #[tokio::test]
    async fn test_schnorr_key_transitions_to_pending_reveal_sign() {
        let ctx = MockWatcherContext::new(true);
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        // Status should carry the sighash from minimal_envelope_data
        assert!(
            matches!(stored.status, L1BundleStatus::PendingRevealTxSign(s) if s == Buf32([42u8; 32]))
        );
        // Envelope is cached for the reveal sig step
        assert!(state.envelope_cache.contains_key(&0));
    }

    #[tokio::test]
    async fn test_schnorr_key_not_enough_utxos_keeps_unsigned_for_retry() {
        let ctx = MockWatcherContext::new(true).with_create_not_enough_utxos();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        // Unsigned entries use zero txids as sentinels because no txs have been built yet.
        assert_eq!(stored.commit_txid, L1TxId::zero());
        assert_eq!(stored.reveal_txid, L1TxId::zero());
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
    }

    #[tokio::test]
    async fn test_schnorr_key_fee_guardrail_keeps_unsigned_for_rebuild() {
        let ctx = MockWatcherContext::new(true).with_create_fee_guardrail();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_unsigned_or_needs_resign(entry).await.unwrap();

        assert_eq!(
            state.ctx.get_stored(0).unwrap().status,
            L1BundleStatus::Unsigned
        );
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
        assert_eq!(state.ctx.rpc_error_count(), 0);
    }

    #[tokio::test]
    async fn test_schnorr_key_prereq_fetch_keeps_unsigned_for_retry() {
        let ctx = MockWatcherContext::new(true).with_create_prereq_fetch();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry);

        let mut state = WatcherState::new(ctx, 0);
        let response = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert!(matches!(response, Response::Continue));
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
        assert_eq!(state.ctx.rpc_error_count(), 1);
    }

    #[tokio::test]
    async fn test_unchecked_sign_raw_transaction_keeps_unsigned_for_retry() {
        let ctx = MockWatcherContext::new(false).with_sign_raw_transaction_failure();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry);

        let mut state = WatcherState::new(ctx, 0);
        let response = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert!(matches!(response, Response::Continue));
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert_eq!(state.curr_payloadidx, 0);
        assert!(state.envelope_cache.is_empty());
        assert_eq!(state.ctx.rpc_error_count(), 1);
    }

    #[tokio::test]
    async fn test_unchecked_other_error_exits_watcher() {
        let ctx = MockWatcherContext::new(false).with_sign_other_failure();
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry);

        let mut state = WatcherState::new(ctx, 0);
        let err = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap_err();

        assert!(err.to_string().contains("mock storage failure"));
        assert_eq!(
            state.ctx.get_stored(0).unwrap().status,
            L1BundleStatus::Unsigned
        );
        assert_eq!(state.ctx.rpc_error_count(), 0);
    }

    #[tokio::test]
    async fn test_signing_mode_error_defers_then_recovers() {
        let ctx = MockWatcherContext::new(true);
        ctx.set_signing_mode_failure(true);
        let entry = test_unsigned_entry();
        ctx.stored.lock().unwrap().insert(0, entry);

        let mut state = WatcherState::new(ctx, 0);

        // An unresolvable signing mode (transient DB error, or a non-signable
        // predicate after a rotation) must defer to the next tick, not kill the
        // writer. The service treats a `process_input` error as terminal.
        let response = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap();
        assert!(matches!(response, Response::Continue));
        assert_eq!(
            state.ctx.get_stored(0).unwrap().status,
            L1BundleStatus::Unsigned
        );
        assert!(state.envelope_cache.is_empty());
        // A signing-mode failure is not a Bitcoin RPC error, so none is reported.
        assert_eq!(state.ctx.rpc_error_count(), 0);

        // Once the signing mode resolves again, the tick loop makes progress.
        state.ctx.set_signing_mode_failure(false);
        let response = WatcherService::<MockWatcherContext>::process_input(&mut state, ())
            .await
            .unwrap();
        assert!(matches!(response, Response::Continue));
        assert!(matches!(
            state.ctx.get_stored(0).unwrap().status,
            L1BundleStatus::PendingRevealTxSign(_)
        ));
    }

    #[tokio::test]
    async fn test_pending_reveal_resets_when_signing_mode_changes_without_sig() {
        let ctx = MockWatcherContext::new(true);

        let envelope = minimal_envelope_data();
        let mut entry = test_unsigned_entry();
        entry.status = L1BundleStatus::PendingRevealTxSign(Buf32([42u8; 32]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.envelope_cache.insert(0, envelope);
        state.ctx.set_signing_mode(EnvelopeSigningMode::InProcess);

        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert!(!state.envelope_cache.contains_key(&0));
    }

    #[tokio::test]
    async fn test_schnorr_key_reveal_sig_transitions_to_unpublished() {
        let ctx = MockWatcherContext::new(true);
        let bcast_handle = ctx.broadcast_handle.clone();

        let envelope = minimal_envelope_data();
        let commit_txid = to_l1_txid(envelope.commit_tx.compute_txid());
        let reveal_txid = to_l1_txid(envelope.reveal_tx.compute_txid());

        // Set up entry already in PendingRevealTxSign with a signature present
        let mut entry = test_unsigned_entry();
        entry.status = L1BundleStatus::PendingRevealTxSign(Buf32([42u8; 32]));
        entry.payload_signature = Some(Buf64([1u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.envelope_cache.insert(0, envelope);

        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unpublished);
        assert_eq!(Buf32(stored.commit_txid.0), commit_txid);
        assert_eq!(Buf32(stored.reveal_txid.0), reveal_txid);
        // Cache entry consumed
        assert!(!state.envelope_cache.contains_key(&0));
        // Both txs stored in broadcaster DB
        assert!(bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(commit_txid))
            .await
            .unwrap()
            .is_some());
        assert!(bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(reveal_txid))
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn test_pending_reveal_resets_when_signing_mode_changes_with_sig() {
        let ctx = MockWatcherContext::new(true);

        let envelope = minimal_envelope_data();
        let commit_txid: Buf32 = envelope.commit_tx.compute_txid().to_buf32();
        let mut entry = test_unsigned_entry();
        entry.status = L1BundleStatus::PendingRevealTxSign(Buf32([42u8; 32]));
        entry.payload_signature = Some(Buf64([1u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.envelope_cache.insert(0, envelope);
        state.ctx.set_signing_mode(EnvelopeSigningMode::InProcess);

        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert!(!state.envelope_cache.contains_key(&0));
        assert!(state
            .ctx
            .broadcast_handle
            .get_tx_entry_by_id_async(commit_txid)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_pending_reveal_cache_miss_resets_to_unsigned() {
        let envelope = minimal_envelope_data();
        let commit_txid = to_l1_txid(envelope.commit_tx.compute_txid());
        let reveal_txid = to_l1_txid(envelope.reveal_tx.compute_txid());
        let ctx = MockWatcherContext::new(true);

        let mut entry = test_unsigned_entry();
        entry.commit_txid = commit_txid;
        entry.reveal_txid = reveal_txid;
        entry.status = L1BundleStatus::PendingRevealTxSign(Buf32([42u8; 32]));
        entry.payload_signature = Some(Buf64([1u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert_eq!(stored.payload_signature, None);
        assert_eq!(stored.commit_txid, L1TxId::from(commit_txid.0));
        assert_eq!(stored.reveal_txid, L1TxId::from(reveal_txid.0));
    }

    #[tokio::test]
    async fn test_pending_reveal_cache_miss_completes_pending_replacement_node() {
        let envelope = minimal_envelope_data();
        let ctx = MockWatcherContext::new(true);
        let bcast_handle = ctx.broadcast_handle.clone();
        let original_signature = [1u8; 64];
        let original_reveal_txid =
            complete_reveal_and_broadcast(0, &envelope, &original_signature, &bcast_handle)
                .await
                .unwrap();
        let original_reveal_entry = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid))
            .await
            .unwrap()
            .expect("original reveal entry exists");
        let original_reveal_tx = original_reveal_entry.try_to_tx().unwrap();
        let reveal_node_id =
            TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx: 0 });
        let mut reveal_record = bcast_handle
            .get_tx_node(reveal_node_id)
            .await
            .unwrap()
            .expect("reveal tx-node exists");
        let (pending_reveal, sighash) = build_pending_single_reveal_replacement(
            &original_reveal_tx,
            &envelope.commit_tx.output[0],
            FeeRate::from_sat_per_vb(2).unwrap(),
            1,
        )
        .unwrap();
        let pending_reveal_txid = pending_reveal.txid;
        reveal_record.append_pending_signature_replacement(pending_reveal);
        bcast_handle.put_tx_node(reveal_record).await.unwrap();

        let mut entry = test_unsigned_entry();
        entry.commit_txid = to_l1_txid(envelope.commit_tx.compute_txid());
        entry.reveal_txid = pending_reveal_txid;
        entry.status = L1BundleStatus::PendingRevealTxSign(sighash);
        entry.payload_signature = Some(Buf64([2u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unpublished);
        assert_eq!(stored.reveal_txid, pending_reveal_txid);
        assert!(bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(pending_reveal_txid))
            .await
            .unwrap()
            .is_some());
        assert!(matches!(
            bcast_handle
                .get_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid))
                .await
                .unwrap()
                .expect("original reveal entry exists")
                .status,
            L1TxStatus::Replaced { .. }
        ));

        let reveal_record = bcast_handle
            .get_tx_node(reveal_node_id)
            .await
            .unwrap()
            .expect("reveal tx-node exists");
        assert_eq!(
            reveal_record.active_attempt().map(|attempt| attempt.status),
            Some(TxAttemptStatus::Active)
        );
    }

    /// Regression: the RPC validates a signature against the sighash on the payload row, and the
    /// fee bumper is a second writer of that row. If the row and the cached envelope describe
    /// different reveals, attaching the signature yields a witness that can never validate, and a
    /// reveal rejected on script grounds never becomes `InvalidInputs`, so nothing rebuilds it.
    #[tokio::test]
    async fn test_reveal_signature_for_a_different_sighash_is_not_attached() {
        let envelope = minimal_envelope_data();
        let ctx = MockWatcherContext::new(true);
        let bcast_handle = ctx.broadcast_handle.clone();

        let mut entry = test_unsigned_entry();
        entry.commit_txid = to_l1_txid(envelope.commit_tx.compute_txid());
        entry.reveal_txid = to_l1_txid(envelope.reveal_tx.compute_txid());
        // A sighash that is not the cached envelope's.
        entry.status = L1BundleStatus::PendingRevealTxSign(Buf32([7u8; 32]));
        entry.payload_signature = Some(Buf64([1u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.envelope_cache.insert(0, envelope.clone());
        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert_eq!(stored.payload_signature, None);
        assert!(
            bcast_handle
                .get_tx_entry_by_id_async(to_raw_buf32(to_l1_txid(
                    envelope.reveal_tx.compute_txid()
                )))
                .await
                .unwrap()
                .is_none(),
            "nothing may reach the broadcaster"
        );
    }

    /// The matching sighash still goes through, so the check does not block the normal path.
    #[tokio::test]
    async fn test_reveal_signature_for_the_cached_sighash_is_attached() {
        let envelope = minimal_envelope_data();
        let ctx = MockWatcherContext::new(true);

        let mut entry = test_unsigned_entry();
        entry.commit_txid = to_l1_txid(envelope.commit_tx.compute_txid());
        entry.reveal_txid = to_l1_txid(envelope.reveal_tx.compute_txid());
        entry.status = L1BundleStatus::PendingRevealTxSign(envelope.sighash);
        entry.payload_signature = Some(Buf64([1u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.envelope_cache.insert(0, envelope);
        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        assert_eq!(
            state.ctx.get_stored(0).unwrap().status,
            L1BundleStatus::Unpublished
        );
    }

    /// Sets up a payload whose published reveal has a pending externally signed fee-bump
    /// replacement, the state the fee bumper leaves behind while it waits on the signer.
    ///
    /// Returns the mock context, the original reveal's txid and the pending replacement's txid.
    /// The payload row names the original reveal, as the fee bumper leaves it: the replacement
    /// only takes its place once its witness is attached.
    async fn pending_reveal_replacement_fixture(
    ) -> (MockWatcherContext, BundledPayloadEntry, L1TxId, L1TxId) {
        let envelope = minimal_envelope_data();
        let ctx = MockWatcherContext::new(true);
        let bcast_handle = ctx.broadcast_handle.clone();
        let original_reveal_txid =
            complete_reveal_and_broadcast(0, &envelope, &[1u8; 64], &bcast_handle)
                .await
                .unwrap();
        let original_reveal_tx = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid))
            .await
            .unwrap()
            .expect("original reveal entry exists")
            .try_to_tx()
            .unwrap();
        let mut reveal_record = bcast_handle
            .get_tx_node(TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal {
                payload_idx: 0,
            }))
            .await
            .unwrap()
            .expect("reveal tx-node exists");
        let (pending_reveal, sighash) = build_pending_single_reveal_replacement(
            &original_reveal_tx,
            &envelope.commit_tx.output[0],
            FeeRate::from_sat_per_vb(2).unwrap(),
            1,
        )
        .unwrap();
        let pending_reveal_txid = pending_reveal.txid;
        reveal_record.append_pending_signature_replacement(pending_reveal);
        bcast_handle.put_tx_node(reveal_record).await.unwrap();

        let mut entry = test_unsigned_entry();
        entry.commit_txid = to_l1_txid(envelope.commit_tx.compute_txid());
        entry.reveal_txid = original_reveal_txid;
        entry.status = L1BundleStatus::PendingRevealTxSign(sighash);
        entry.payload_signature = Some(Buf64([2u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        (ctx, entry, original_reveal_txid, pending_reveal_txid)
    }

    async fn reveal_record(bcast_handle: &L1BroadcastHandle) -> TxNodeRecord {
        bcast_handle
            .get_tx_node(TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal {
                payload_idx: 0,
            }))
            .await
            .unwrap()
            .expect("reveal tx-node exists")
    }

    /// Regression: a replacement reuses the original reveal's tapscript, so it can only be
    /// completed under the key that script commits to. The signing RPC verifies against whatever
    /// key is canonical *now*, so after a rotation it accepts a signature that cannot satisfy the
    /// old script — attaching it would supersede a valid reveal with an unspendable one.
    #[tokio::test]
    async fn test_pending_reveal_replacement_refused_when_signing_key_rotates() {
        let (ctx, entry, original_reveal_txid, pending_reveal_txid) =
            pending_reveal_replacement_fixture().await;
        let bcast_handle = ctx.broadcast_handle.clone();
        ctx.set_signing_mode(EnvelopeSigningMode::External {
            pubkey: minimal_envelope_data_for(&[2u8; 32]).envelope_pubkey,
        });

        let mut state = WatcherState::new(ctx, 0);
        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        assert!(
            bcast_handle
                .get_tx_entry_by_id_async(to_raw_buf32(pending_reveal_txid))
                .await
                .unwrap()
                .is_none(),
            "the replacement must never reach the broadcaster"
        );
        assert_eq!(
            bcast_handle
                .get_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid))
                .await
                .unwrap()
                .expect("original reveal entry exists")
                .status,
            L1TxStatus::Unpublished,
            "the original must not be superseded"
        );

        let record = reveal_record(&bcast_handle).await;
        assert_eq!(
            record.terminal_error,
            Some(TerminalError::UnsupportedRbfKind)
        );
        assert_eq!(
            record
                .attempts
                .iter()
                .find(|attempt| attempt.txid == pending_reveal_txid)
                .map(|attempt| attempt.status),
            Some(TxAttemptStatus::Discarded)
        );

        // The original is still live, so the payload tracks it rather than being rebuilt.
        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unpublished);
        assert_eq!(stored.reveal_txid, original_reveal_txid);
    }

    /// Regression: only `Unpublished` and `Published` entries are replaceable, so an original that
    /// went invalid refuses the swap. The pending attempt is dead at that point, and leaving it
    /// durable pins the payload in `PendingRevealTxSign` forever.
    #[tokio::test]
    async fn test_pending_reveal_replacement_discarded_when_original_is_invalid() {
        let (ctx, entry, original_reveal_txid, pending_reveal_txid) =
            pending_reveal_replacement_fixture().await;
        let bcast_handle = ctx.broadcast_handle.clone();
        let mut original_reveal_entry = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid))
            .await
            .unwrap()
            .expect("original reveal entry exists");
        original_reveal_entry.status = L1TxStatus::InvalidInputs;
        bcast_handle
            .update_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid), original_reveal_entry)
            .await
            .unwrap();

        let mut state = WatcherState::new(ctx, 0);
        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let record = reveal_record(&bcast_handle).await;
        assert_eq!(record.terminal_error, None, "the chain is still bumpable");
        assert_eq!(
            record
                .attempts
                .iter()
                .find(|attempt| attempt.txid == pending_reveal_txid)
                .map(|attempt| attempt.status),
            Some(TxAttemptStatus::Discarded)
        );

        // Nothing spendable is left, so the payload rebuilds from fresh UTXOs.
        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert_eq!(stored.payload_signature, None);
    }

    /// Regression: the signature may never arrive. If the original reveal goes invalid first, the
    /// no-signature path used to return early on the pending attempt every tick, so the payload sat
    /// in `PendingRevealTxSign` forever — the fee bumper skips nodes carrying a pending attempt, so
    /// nothing else would move it either.
    #[tokio::test]
    async fn test_pending_reveal_replacement_without_signature_rebuilds_when_original_is_invalid() {
        let (ctx, mut entry, original_reveal_txid, pending_reveal_txid) =
            pending_reveal_replacement_fixture().await;
        let bcast_handle = ctx.broadcast_handle.clone();
        entry.payload_signature = None;
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut original_reveal_entry = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid))
            .await
            .unwrap()
            .expect("original reveal entry exists");
        original_reveal_entry.status = L1TxStatus::InvalidInputs;
        bcast_handle
            .update_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid), original_reveal_entry)
            .await
            .unwrap();

        let mut state = WatcherState::new(ctx, 0);
        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let record = reveal_record(&bcast_handle).await;
        assert_eq!(
            record
                .attempts
                .iter()
                .find(|attempt| attempt.txid == pending_reveal_txid)
                .map(|attempt| attempt.status),
            Some(TxAttemptStatus::Discarded),
            "the replacement can never be broadcast, so it must not keep blocking the node"
        );
        assert!(
            bcast_handle
                .get_tx_entry_by_id_async(to_raw_buf32(pending_reveal_txid))
                .await
                .unwrap()
                .is_none(),
            "the replacement must never reach the broadcaster"
        );

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unsigned);
        assert_eq!(stored.payload_signature, None);
    }

    /// The same path must keep waiting while the original is still live, or a fee bump would
    /// double-publish the payload.
    #[tokio::test]
    async fn test_pending_reveal_replacement_without_signature_waits_on_a_live_original() {
        let (ctx, mut entry, _, pending_reveal_txid) = pending_reveal_replacement_fixture().await;
        let bcast_handle = ctx.broadcast_handle.clone();
        entry.payload_signature = None;
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state
            .handle_pending_reveal_tx_sign(entry.clone())
            .await
            .unwrap();

        let record = reveal_record(&bcast_handle).await;
        assert_eq!(
            record
                .attempts
                .iter()
                .find(|attempt| attempt.txid == pending_reveal_txid)
                .map(|attempt| attempt.status),
            Some(TxAttemptStatus::PendingSignature)
        );
        assert_eq!(state.ctx.get_stored(0).unwrap().status, entry.status);
    }

    /// Regression: a stop between broadcasting an envelope and writing its payload row leaves the
    /// row in `PendingRevealTxSign` with both transactions already queued. Rebuilding there draws
    /// fresh UTXOs and publishes the same payload twice.
    #[tokio::test]
    async fn test_pending_reveal_cache_miss_adopts_live_broadcast_state() {
        let envelope = minimal_envelope_data();
        let ctx = MockWatcherContext::new(true);
        let bcast_handle = ctx.broadcast_handle.clone();
        let reveal_txid = complete_reveal_and_broadcast(0, &envelope, &[1u8; 64], &bcast_handle)
            .await
            .unwrap();

        let mut entry = test_unsigned_entry();
        entry.commit_txid = to_l1_txid(envelope.commit_tx.compute_txid());
        entry.reveal_txid = reveal_txid;
        entry.status = L1BundleStatus::PendingRevealTxSign(Buf32([42u8; 32]));
        entry.payload_signature = Some(Buf64([1u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Unpublished);
        assert_eq!(stored.reveal_txid, reveal_txid);
    }

    #[tokio::test]
    async fn test_pending_reveal_signature_keeps_confirmed_original() {
        let envelope = minimal_envelope_data();
        let ctx = MockWatcherContext::new(true);
        let bcast_handle = ctx.broadcast_handle.clone();
        let original_signature = [1u8; 64];
        let original_reveal_txid =
            complete_reveal_and_broadcast(0, &envelope, &original_signature, &bcast_handle)
                .await
                .unwrap();
        let mut original_reveal_entry = bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid))
            .await
            .unwrap()
            .expect("original reveal entry exists");
        let original_reveal_tx = original_reveal_entry.try_to_tx().unwrap();
        let reveal_node_id =
            TxNodeId::from_kind(&TxNodeKind::SingleEnvelopeReveal { payload_idx: 0 });
        let mut reveal_record = bcast_handle
            .get_tx_node(reveal_node_id)
            .await
            .unwrap()
            .expect("reveal tx-node exists");
        let (pending_reveal, sighash) = build_pending_single_reveal_replacement(
            &original_reveal_tx,
            &envelope.commit_tx.output[0],
            FeeRate::from_sat_per_vb(2).unwrap(),
            1,
        )
        .unwrap();
        let pending_reveal_txid = pending_reveal.txid;
        reveal_record.append_pending_signature_replacement(pending_reveal);
        bcast_handle.put_tx_node(reveal_record).await.unwrap();

        original_reveal_entry.status = L1TxStatus::Confirmed {
            confirmations: 1,
            block_hash: Buf32::zero(),
            block_height: 100,
        };
        bcast_handle
            .update_tx_entry_by_id_async(to_raw_buf32(original_reveal_txid), original_reveal_entry)
            .await
            .unwrap();

        let mut entry = test_unsigned_entry();
        entry.commit_txid = to_l1_txid(envelope.commit_tx.compute_txid());
        entry.reveal_txid = original_reveal_txid;
        entry.status = L1BundleStatus::PendingRevealTxSign(sighash);
        entry.payload_signature = Some(Buf64([2u8; 64]));
        ctx.stored.lock().unwrap().insert(0, entry.clone());

        let mut state = WatcherState::new(ctx, 0);
        state.handle_pending_reveal_tx_sign(entry).await.unwrap();

        let stored = state.ctx.get_stored(0).unwrap();
        assert_eq!(stored.status, L1BundleStatus::Confirmed);
        assert_eq!(stored.reveal_txid, original_reveal_txid);
        assert!(bcast_handle
            .get_tx_entry_by_id_async(to_raw_buf32(pending_reveal_txid))
            .await
            .unwrap()
            .is_none());

        let reveal_record = bcast_handle
            .get_tx_node(reveal_node_id)
            .await
            .unwrap()
            .expect("reveal tx-node exists");
        assert_eq!(
            reveal_record.active_attempt().map(|attempt| attempt.status),
            Some(TxAttemptStatus::Active)
        );
        assert_eq!(
            reveal_record
                .attempts
                .iter()
                .find(|attempt| attempt.txid == pending_reveal_txid)
                .map(|attempt| attempt.status),
            Some(TxAttemptStatus::Discarded)
        );
    }
}
