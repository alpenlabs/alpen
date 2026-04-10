//! Watcher service for the btcio L1 writer.
//!
//! Drives the [`L1BundleStatus`] state machine for the current payload entry
//! on each timer tick.

use std::{collections::HashMap, future::Future, marker::PhantomData, sync::Arc};

use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use serde::Serialize;
use strata_btc_types::{Buf32BitcoinExt, TxidExt};
use strata_db_types::types::{BundledPayloadEntry, L1BundleStatus, L1TxEntry, L1TxStatus};
use strata_primitives::buf::Buf32;
use strata_service::{AsyncService, Response, Service, ServiceState};
use strata_status::StatusChannel;
use strata_storage::ops::writer::EnvelopeDataOps;
use tracing::*;

use crate::{
    broadcaster::L1BroadcastHandle,
    status::{apply_status_updates, L1StatusUpdate},
    writer::{
        builder::{EnvelopeData, EnvelopeError},
        context::WriterContext,
        signer::{complete_reveal_and_broadcast, create_payload_envelopes},
    },
};

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
    fn create_envelopes(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
    ) -> impl Future<Output = Result<EnvelopeData, EnvelopeError>> + Send;
    fn complete_reveal_and_broadcast(
        &self,
        idx: u64,
        envelope: &EnvelopeData,
        sig: &[u8; 64],
    ) -> impl Future<Output = anyhow::Result<Buf32>> + Send;
    fn get_tx_status(
        &self,
        txid: Buf32,
    ) -> impl Future<Output = anyhow::Result<Option<L1TxEntry>>> + Send;
    fn report_status(
        &self,
        entry: &BundledPayloadEntry,
        status: &L1BundleStatus,
    ) -> impl Future<Output = ()> + Send;
}

pub(crate) struct WatcherContextImpl<R: Reader + Signer + Wallet + Send + Sync + 'static> {
    context: Arc<WriterContext<R>>,
    ops: Arc<EnvelopeDataOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> WatcherContextImpl<R> {
    pub(crate) fn new(
        context: Arc<WriterContext<R>>,
        ops: Arc<EnvelopeDataOps>,
        broadcast_handle: Arc<L1BroadcastHandle>,
    ) -> Self {
        Self {
            context,
            ops,
            broadcast_handle,
        }
    }
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> WatcherServiceContext
    for WatcherContextImpl<R>
{
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

    async fn create_envelopes(
        &self,
        idx: u64,
        entry: &BundledPayloadEntry,
    ) -> Result<EnvelopeData, EnvelopeError> {
        create_payload_envelopes(idx, entry, self.context.clone()).await
    }

    async fn complete_reveal_and_broadcast(
        &self,
        idx: u64,
        envelope: &EnvelopeData,
        sig: &[u8; 64],
    ) -> anyhow::Result<Buf32> {
        complete_reveal_and_broadcast(idx, envelope, sig, &self.broadcast_handle)
            .await
            .map_err(Into::into)
    }

    async fn get_tx_status(&self, txid: Buf32) -> anyhow::Result<Option<L1TxEntry>> {
        self.broadcast_handle
            .get_tx_entry_by_id_async(txid)
            .await
            .map_err(Into::into)
    }

    async fn report_status(&self, entry: &BundledPayloadEntry, status: &L1BundleStatus) {
        update_l1_status(entry, status, &self.context.status_channel).await;
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
                L1BundleStatus::Finalized => {
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
    /// Builds envelope txs and transitions to `PendingRevealTxSign`.
    ///
    /// Signs the commit tx via wallet and caches the envelope. The reveal tx
    /// awaits an external Schnorr signature before broadcast.
    async fn handle_unsigned_or_needs_resign(
        &mut self,
        payloadentry: BundledPayloadEntry,
    ) -> anyhow::Result<()> {
        debug!(current_status=?payloadentry.status);
        match self
            .ctx
            .create_envelopes(self.curr_payloadidx, &payloadentry)
            .await
        {
            Ok(envelope) => {
                let cid: Buf32 = envelope.commit_tx.compute_txid().to_buf32();
                let rid: Buf32 = envelope.reveal_tx.compute_txid().to_buf32();
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
                // Just wait till we have enough utxos and let the status be
                // `Unsigned` or `NeedsResign`
                error!(%required, %available, "not enough utxos to create commit/reveal transaction");
            }
            e => {
                e?;
            }
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
            trace!("waiting for signer to provide reveal signature");
            return Ok(());
        };
        let Some(envelope) = self.envelope_cache.remove(&self.curr_payloadidx) else {
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
        match self
            .ctx
            .complete_reveal_and_broadcast(self.curr_payloadidx, &envelope, sig.as_ref())
            .await
        {
            Ok(_rid) => {
                let mut updated_entry = payloadentry.clone();
                updated_entry.status = L1BundleStatus::Unpublished;
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;
                debug!("reveal signed and stored for broadcast");
            }
            Err(e) => {
                error!(%e, "failed to attach reveal signature");
            }
        }
        Ok(())
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
            (Some(ctx), Some(rtx)) => {
                let new_status = determine_payload_next_status(&ctx.status, &rtx.status);
                debug!(?new_status, "The next status for payload");
                if matches!(
                    new_status,
                    L1BundleStatus::Confirmed | L1BundleStatus::Finalized
                ) {
                    info!(
                        component = "btcio_writer",
                        payload_idx = self.curr_payloadidx,
                        commit_txid = %payloadentry.commit_txid,
                        reveal_txid = %payloadentry.reveal_txid,
                        payload_status = ?new_status,
                        commit_l1_status = ?ctx.status,
                        reveal_l1_status = ?rtx.status,
                        "payload advanced on L1"
                    );
                }

                self.ctx.report_status(&payloadentry, &new_status).await;

                // Update payloadentry with new status
                let mut updated_entry = payloadentry.clone();
                updated_entry.status = new_status.clone();
                self.ctx
                    .put_payload_entry(self.curr_payloadidx, updated_entry)
                    .await?;

                if new_status == L1BundleStatus::Finalized {
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
    if *new_status == L1BundleStatus::Published
        || *new_status == L1BundleStatus::Confirmed
        || *new_status == L1BundleStatus::Finalized
    {
        let status_updates = [
            L1StatusUpdate::LastPublishedTxid(payloadentry.reveal_txid.to_txid()),
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
        // if commit has invalid inputs, needs resign
        (L1TxStatus::InvalidInputs, _) => L1BundleStatus::NeedsResign,
        // If commit is unpublished, both are upublished
        (L1TxStatus::Unpublished, _) => L1BundleStatus::Unpublished,
        // If commit is published but not reveal, the payload is unpublished
        (_, L1TxStatus::Unpublished) => L1BundleStatus::Unpublished,
        // If reveal has invalid inputs, these need resign because we can do nothing with just
        // commit tx confirmed. This should not occur in practice
        (_, L1TxStatus::InvalidInputs) => L1BundleStatus::NeedsResign,
    }
}
