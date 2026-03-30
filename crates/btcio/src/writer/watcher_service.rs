//! Watcher service for the btcio L1 writer.
//!
//! Drives the [`L1BundleStatus`] state machine for the current payload entry
//! on each timer tick.

use std::{collections::HashMap, marker::PhantomData, sync::Arc, time::Duration};

use bitcoind_async_client::traits::{Reader, Signer, Wallet};
use serde::Serialize;
use strata_btc_types::{Buf32BitcoinExt, TxidExt};
use strata_db_types::types::{BundledPayloadEntry, L1BundleStatus, L1TxStatus};
use strata_primitives::buf::Buf32;
use strata_service::{
    AsyncService, AsyncServiceInput, Response, Service, ServiceInput, ServiceState,
};
use strata_status::StatusChannel;
use strata_storage::ops::writer::EnvelopeDataOps;
use tokio::time::{interval, Interval};
use tracing::*;

use crate::{
    broadcaster::L1BroadcastHandle,
    status::{apply_status_updates, L1StatusUpdate},
    writer::{
        builder::{EnvelopeError, UnsignedEnvelopeData},
        context::WriterContext,
        signer::{complete_reveal_and_broadcast, create_payload_envelopes},
    },
};

#[derive(Debug)]
pub(crate) enum WatcherEvent {
    PollTick,
}

pub(crate) struct WatcherInput {
    interval: Interval,
}

impl WatcherInput {
    pub(crate) fn new(poll_dur: Duration) -> Self {
        Self {
            interval: interval(poll_dur),
        }
    }
}

impl ServiceInput for WatcherInput {
    type Msg = WatcherEvent;
}

impl AsyncServiceInput for WatcherInput {
    async fn recv_next(&mut self) -> anyhow::Result<Option<WatcherEvent>> {
        self.interval.tick().await;
        Ok(Some(WatcherEvent::PollTick))
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct WatcherStatus {
    pub(crate) current_payload_idx: u64,
    pub(crate) cache_size: usize,
}

pub(crate) struct WatcherState<R: Reader + Signer + Wallet + Send + Sync + 'static> {
    pub(crate) context: Arc<WriterContext<R>>,
    pub(crate) ops: Arc<EnvelopeDataOps>,
    pub(crate) broadcast_handle: Arc<L1BroadcastHandle>,
    pub(crate) unsigned_cache: HashMap<u64, UnsignedEnvelopeData>,
    pub(crate) curr_payloadidx: u64,
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> ServiceState for WatcherState<R> {
    fn name(&self) -> &str {
        "btcio_watcher"
    }
}

pub(crate) struct WatcherService<R>(PhantomData<R>);

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> Service for WatcherService<R> {
    type State = WatcherState<R>;
    type Msg = WatcherEvent;
    type Status = WatcherStatus;

    fn get_status(state: &Self::State) -> Self::Status {
        WatcherStatus {
            current_payload_idx: state.curr_payloadidx,
            cache_size: state.unsigned_cache.len(),
        }
    }
}

impl<R: Reader + Signer + Wallet + Send + Sync + 'static> AsyncService for WatcherService<R> {
    async fn process_input(state: &mut Self::State, _input: Self::Msg) -> anyhow::Result<Response> {
        let dspan = debug_span!("process payload", idx=%state.curr_payloadidx);
        let _ = dspan.enter();

        if let Some(payloadentry) = state
            .ops
            .get_payload_entry_by_idx_async(state.curr_payloadidx)
            .await?
        {
            match payloadentry.status {
                // If unsigned or needs resign, build envelope txs, sign commit with
                // wallet, and transition to PendingPayloadSign awaiting the external
                // signer's Schnorr signature on the reveal tx.
                L1BundleStatus::Unsigned | L1BundleStatus::NeedsResign => {
                    debug!(current_status=?payloadentry.status);
                    match create_payload_envelopes(
                        state.curr_payloadidx,
                        &payloadentry,
                        &state.broadcast_handle,
                        state.context.clone(),
                    )
                    .await
                    {
                        Ok((unsigned, cid)) => {
                            let rid: Buf32 = unsigned.reveal_tx.compute_txid().to_buf32();
                            let sighash = unsigned.sighash;

                            let mut updated_entry = payloadentry.clone();
                            updated_entry.commit_txid = cid;
                            updated_entry.reveal_txid = rid;
                            updated_entry.payload_signature = None;
                            updated_entry.status = L1BundleStatus::PendingPayloadSign(sighash);
                            state
                                .ops
                                .put_payload_entry_async(state.curr_payloadidx, updated_entry)
                                .await?;

                            // Cache the unsigned envelope for later signature attachment
                            state.unsigned_cache.insert(state.curr_payloadidx, unsigned);

                            debug!(%sighash, "envelope built, awaiting signer");
                        }
                        Err(EnvelopeError::NotEnoughUtxos(required, available)) => {
                            // Just wait till we have enough utxos and let the status be
                            // `Unsigned` or `NeedsResign`
                            // Maybe send an alert
                            error!(%required, %available, "Not enough utxos available to create commit/reveal transaction");
                        }
                        e => {
                            e?;
                        }
                    }
                }

                // Waiting for the external signer to provide the reveal signature.
                // When the signature arrives (via RPC), complete the reveal tx and
                // transition to Unpublished.
                L1BundleStatus::PendingPayloadSign(_sighash) => {
                    let Some(sig) = &payloadentry.payload_signature else {
                        trace!("waiting for signer to provide reveal signature");
                        return Ok(Response::Continue);
                    };
                    let Some(unsigned) = state.unsigned_cache.remove(&state.curr_payloadidx) else {
                        // Cache miss (e.g. restart) — reset to Unsigned to rebuild
                        // envelope from scratch (new UTXOs, new sighash).
                        // Same recovery path as NeedsResign.
                        warn!("unsigned envelope not in cache, resetting to Unsigned");
                        let mut updated_entry = payloadentry.clone();
                        updated_entry.payload_signature = None;
                        updated_entry.status = L1BundleStatus::Unsigned;
                        state
                            .ops
                            .put_payload_entry_async(state.curr_payloadidx, updated_entry)
                            .await?;
                        return Ok(Response::Continue);
                    };
                    match complete_reveal_and_broadcast(
                        state.curr_payloadidx,
                        &unsigned,
                        sig.as_ref(),
                        &state.broadcast_handle,
                    )
                    .await
                    {
                        Ok(_rid) => {
                            let mut updated_entry = payloadentry.clone();
                            updated_entry.status = L1BundleStatus::Unpublished;
                            state
                                .ops
                                .put_payload_entry_async(state.curr_payloadidx, updated_entry)
                                .await?;
                            debug!("reveal signed and stored for broadcast");
                        }
                        Err(e) => {
                            error!(%e, "failed to attach reveal signature");
                        }
                    }
                }
                // If finalized, nothing to do, move on to process next entry
                L1BundleStatus::Finalized => {
                    state.curr_payloadidx += 1;
                }
                // If entry is signed but not finalized or excluded yet, check broadcast txs status
                L1BundleStatus::Published
                | L1BundleStatus::Confirmed
                | L1BundleStatus::Unpublished => {
                    trace!("Checking payloadentry's broadcast status");
                    let commit_tx = state
                        .broadcast_handle
                        .get_tx_entry_by_id_async(payloadentry.commit_txid)
                        .await?;
                    let reveal_tx = state
                        .broadcast_handle
                        .get_tx_entry_by_id_async(payloadentry.reveal_txid)
                        .await?;

                    match (commit_tx, reveal_tx) {
                        (Some(ctx), Some(rtx)) => {
                            let new_status =
                                determine_payload_next_status(&ctx.status, &rtx.status);
                            debug!(?new_status, "The next status for payload");
                            if matches!(
                                new_status,
                                L1BundleStatus::Confirmed | L1BundleStatus::Finalized
                            ) {
                                info!(
                                    component = "btcio_writer",
                                    payload_idx = state.curr_payloadidx,
                                    commit_txid = %payloadentry.commit_txid,
                                    reveal_txid = %payloadentry.reveal_txid,
                                    payload_status = ?new_status,
                                    commit_l1_status = ?ctx.status,
                                    reveal_l1_status = ?rtx.status,
                                    "payload advanced on L1"
                                );
                            }

                            update_l1_status(
                                &payloadentry,
                                &new_status,
                                &state.context.status_channel,
                            )
                            .await;

                            // Update payloadentry with new status
                            let mut updated_entry = payloadentry.clone();
                            updated_entry.status = new_status.clone();
                            state
                                .ops
                                .put_payload_entry_async(state.curr_payloadidx, updated_entry)
                                .await?;

                            if new_status == L1BundleStatus::Finalized {
                                state.curr_payloadidx += 1;
                            }
                        }
                        _ => {
                            warn!("Corresponding commit/reveal entry for payloadentry not found in broadcast db. Sign and create transactions again.");
                            let mut updated_entry = payloadentry.clone();
                            updated_entry.payload_signature = None;
                            updated_entry.status = L1BundleStatus::Unsigned;
                            state
                                .ops
                                .put_payload_entry_async(state.curr_payloadidx, updated_entry)
                                .await?;
                        }
                    }
                }
            }
        } else {
            // No payload exists, just continue the loop to wait for payload's presence in db
            debug!("Waiting for payloadentry to be present in db");
        }

        Ok(Response::Continue)
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
pub(super) fn determine_payload_next_status(
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
