use std::{sync::Arc, time::Duration};

use bitcoin::Address;
use bitcoind_async_client::{
    traits::{Reader, Signer, Wallet},
    Client,
};
use strata_config::btcio::WriterConfig;
use strata_db::{
    traits::L1WriterDatabase,
    types::{BundledPayloadEntry, IntentEntry, L1BundleStatus, L1TxStatus},
};
use strata_primitives::{
    l1::payload::{PayloadDest, PayloadIntent},
    params::Params,
};
use strata_status::StatusChannel;
use strata_storage::ops::writer::{Context, EnvelopeDataOps};
use strata_tasks::TaskExecutor;
use tokio::sync::mpsc::{self, Sender};
use tracing::*;

use super::bundler::{bundler_task, get_initial_unbundled_entries};
use crate::{
    broadcaster::L1BroadcastHandle,
    status::{apply_status_updates, L1StatusUpdate},
    writer::{
        builder::EnvelopeError, context::WriterContext, signer::create_and_sign_payload_envelopes,
    },
};

/// A handle to the Envelope task.
#[expect(missing_debug_implementations)]
pub struct EnvelopeHandle {
    ops: Arc<EnvelopeDataOps>,
    intent_tx: Sender<IntentEntry>,
}

impl EnvelopeHandle {
    #[expect(clippy::missing_const_for_fn)]
    pub fn new(ops: Arc<EnvelopeDataOps>, intent_tx: Sender<IntentEntry>) -> Self {
        Self { ops, intent_tx }
    }

    /// Checks if it is duplicate, if not creates a new [`IntentEntry`] from `intent` and puts it in
    /// the database.
    pub fn submit_intent(&self, intent: PayloadIntent) -> anyhow::Result<()> {
        let id = *intent.commitment();

        // Check if the intent is meant for L1
        if intent.dest() != PayloadDest::L1 {
            warn!(commitment = %id, "Received intent not meant for L1");
            return Ok(());
        }

        debug!(commitment = %id, "Received intent for processing");

        // Check if it is duplicate
        if self.ops.get_intent_by_id_blocking(id)?.is_some() {
            warn!(commitment = %id, "Received duplicate intent");
            return Ok(());
        }

        // Create and store IntentEntry
        let entry = IntentEntry::new_unbundled(intent);
        self.ops.put_intent_entry_blocking(id, entry.clone())?;

        // Send to bundler
        if let Err(e) = self.intent_tx.blocking_send(entry) {
            warn!("Could not send intent entry to bundler: {:?}", e);
        }
        Ok(())
    }

    /// Checks if it is duplicate, if not creates a new [`IntentEntry`] from `intent` and puts it in
    /// the database
    pub async fn submit_intent_async(&self, intent: PayloadIntent) -> anyhow::Result<()> {
        let id = *intent.commitment();

        // Check if the intent is meant for L1
        if intent.dest() != PayloadDest::L1 {
            warn!(commitment = %id, "Received intent not meant for L1");
            return Ok(());
        }

        debug!(commitment = %id, "Received intent for processing");

        // Check if it is duplicate
        if self.ops.get_intent_by_id_async(id).await?.is_some() {
            warn!(commitment = %id, "Received duplicate intent");
            return Ok(());
        }

        // Create and store IntentEntry
        let entry = IntentEntry::new_unbundled(intent);
        self.ops.put_intent_entry_blocking(id, entry.clone())?;

        // Send to bundler
        if let Err(e) = self.intent_tx.send(entry).await {
            warn!("Could not send intent entry to bundler: {:?}", e);
        }

        Ok(())
    }
}

/// Starts the envelope task.
///
/// This creates an [`EnvelopeHandle`] and spawns a watcher task that watches the status of
/// incriptions in bitcoin.
///
/// # Returns
///
/// [`Result<EnvelopeHandle>`](anyhow::Result)
#[allow(clippy::too_many_arguments)]
pub fn start_envelope_task<D: L1WriterDatabase + Send + Sync + 'static>(
    executor: &TaskExecutor,
    bitcoin_client: Arc<Client>,
    config: Arc<WriterConfig>,
    params: Arc<Params>,
    sequencer_address: Address,
    db: Arc<D>,
    status_channel: StatusChannel,
    pool: threadpool::ThreadPool,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> anyhow::Result<Arc<EnvelopeHandle>> {
    let writer_ops = Arc::new(Context::new(db).into_ops(pool));
    let next_watch_payload_idx = get_next_payloadidx_to_watch(writer_ops.as_ref())?;
    let (intent_tx, intent_rx) = mpsc::channel::<IntentEntry>(64);

    let envelope_handle = Arc::new(EnvelopeHandle::new(writer_ops.clone(), intent_tx));
    let ctx = Arc::new(WriterContext::new(
        params,
        config.clone(),
        sequencer_address,
        bitcoin_client,
        status_channel,
    ));

    let wops = writer_ops.clone();
    executor.spawn_critical_async("btcio::watcher_task", async move {
        watcher_task(next_watch_payload_idx, ctx, wops.clone(), broadcast_handle).await
    });

    let unbundled = get_initial_unbundled_entries(writer_ops.as_ref())?;
    executor.spawn_critical_async_with_shutdown("btcio::bundler_task", |shutdown| async move {
        bundler_task(unbundled, writer_ops, config.clone(), intent_rx, shutdown).await
    });

    Ok(envelope_handle)
}

/// Looks into the database from descending index order till it reaches 0 or `Finalized`
/// [`PayloadEntry`] from which the rest of the [`PayloadEntry`]s should be watched.
fn get_next_payloadidx_to_watch(insc_ops: &EnvelopeDataOps) -> anyhow::Result<u64> {
    let mut next_idx = insc_ops.get_next_payload_idx_blocking()?;

    while next_idx > 0 {
        let Some(payload) = insc_ops.get_payload_entry_by_idx_blocking(next_idx - 1)? else {
            break;
        };
        if payload.status == L1BundleStatus::Finalized {
            break;
        };
        next_idx -= 1;
    }
    Ok(next_idx)
}

/// Watches for envelope transactions status in bitcoin. Note that this watches for each
/// envelope until it is confirmed
/// Watches for envelope transactions status in the Bitcoin blockchain.
///
/// # Note
///
/// The envelope will be monitored until it acquires the status of
/// [`BlobL1Status::Finalized`]
pub(crate) async fn watcher_task<R: Reader + Signer + Wallet>(
    next_watch_payload_idx: u64,
    context: Arc<WriterContext<R>>,
    insc_ops: Arc<EnvelopeDataOps>,
    broadcast_handle: Arc<L1BroadcastHandle>,
) -> anyhow::Result<()> {
    info!("Starting L1 writer's watcher task");
    let interval = tokio::time::interval(Duration::from_millis(context.config.write_poll_dur_ms));
    tokio::pin!(interval);

    let mut curr_payloadidx = next_watch_payload_idx;
    loop {
        interval.as_mut().tick().await;

        let dspan = debug_span!("process payload", idx=%curr_payloadidx);
        let _ = dspan.enter();

        if let Some(payloadentry) = insc_ops
            .get_payload_entry_by_idx_async(curr_payloadidx)
            .await?
        {
            match payloadentry.status {
                // If unsigned or needs resign, create new signed commit/reveal txs and update the
                // entry
                L1BundleStatus::Unsigned | L1BundleStatus::NeedsResign => {
                    debug!(current_status=?payloadentry.status);
                    match create_and_sign_payload_envelopes(
                        &payloadentry,
                        &broadcast_handle,
                        context.clone(),
                    )
                    .await
                    {
                        Ok((cid, rid)) => {
                            let mut updated_entry = payloadentry.clone();
                            updated_entry.status = L1BundleStatus::Unpublished;
                            updated_entry.commit_txid = cid;
                            updated_entry.reveal_txid = rid;
                            insc_ops
                                .put_payload_entry_async(curr_payloadidx, updated_entry)
                                .await?;

                            debug!("Signed payload");
                        }
                        Err(EnvelopeError::NotEnoughUtxos(required, available)) => {
                            // Just wait till we have enough utxos and let the status be `Unsigned`
                            // or `NeedsResign`
                            // Maybe send an alert
                            error!(%required, %available, "Not enough utxos available to create commit/reveal transaction");
                        }
                        e => {
                            e?;
                        }
                    }
                }
                // If finalized, nothing to do, move on to process next entry
                L1BundleStatus::Finalized => {
                    curr_payloadidx += 1;
                }
                // If entry is signed but not finalized or excluded yet, check broadcast txs status
                L1BundleStatus::Published
                | L1BundleStatus::Confirmed
                | L1BundleStatus::Unpublished => {
                    trace!("Checking payloadentry's broadcast status");
                    let commit_tx = broadcast_handle
                        .get_tx_entry_by_id_async(payloadentry.commit_txid)
                        .await?;
                    let reveal_tx = broadcast_handle
                        .get_tx_entry_by_id_async(payloadentry.reveal_txid)
                        .await?;

                    match (commit_tx, reveal_tx) {
                        (Some(ctx), Some(rtx)) => {
                            let new_status =
                                determine_payload_next_status(&ctx.status, &rtx.status);
                            debug!(?new_status, "The next status for payload");

                            update_l1_status(&payloadentry, &new_status, &context.status_channel)
                                .await;

                            // Update payloadentry with new status
                            let mut updated_entry = payloadentry.clone();
                            updated_entry.status = new_status.clone();
                            insc_ops
                                .put_payload_entry_async(curr_payloadidx, updated_entry)
                                .await?;

                            if new_status == L1BundleStatus::Finalized {
                                curr_payloadidx += 1;
                            }
                        }
                        _ => {
                            warn!("Corresponding commit/reveal entry for payloadentry not found in broadcast db. Sign and create transactions again.");
                            let mut updated_entry = payloadentry.clone();
                            updated_entry.status = L1BundleStatus::Unsigned;
                            insc_ops
                                .put_payload_entry_async(curr_payloadidx, updated_entry)
                                .await?;
                        }
                    }
                }
            }
        } else {
            // No payload exists, just continue the loop to wait for payload's presence in db
            debug!("Waiting for payloadentry to be present in db");
        }
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
            L1StatusUpdate::LastPublishedTxid(payloadentry.reveal_txid.into()),
            L1StatusUpdate::IncrementPublishedRevealCount,
        ];
        apply_status_updates(&status_updates, status_channel).await;
    }
}

/// Determine the status of the `PayloadEntry` based on the status of its commit and reveal
/// transactions in bitcoin.
const fn determine_payload_next_status(
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

#[cfg(test)]
mod test {
    use strata_test_utils::ArbitraryGenerator;

    use super::*;
    use crate::writer::test_utils::get_envelope_ops;

    #[test]
    fn test_initialize_writer_state_no_last_payload_idx() {
        let iops = get_envelope_ops();

        let nextidx = iops.get_next_payload_idx_blocking().unwrap();
        assert_eq!(nextidx, 0);

        let idx = get_next_payloadidx_to_watch(&iops).unwrap();

        assert_eq!(idx, 0);
    }

    #[test]
    fn test_initialize_writer_state_with_existing_payloads() {
        let iops = get_envelope_ops();

        let mut e1: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        e1.status = L1BundleStatus::Finalized;
        iops.put_payload_entry_blocking(0, e1).unwrap();

        let mut e2: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        e2.status = L1BundleStatus::Published;
        iops.put_payload_entry_blocking(1, e2).unwrap();
        let expected_idx = 1; // All entries before this do not need to be watched.

        let mut e3: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        e3.status = L1BundleStatus::Unsigned;
        iops.put_payload_entry_blocking(2, e3).unwrap();

        let mut e4: BundledPayloadEntry = ArbitraryGenerator::new().generate();
        e4.status = L1BundleStatus::Unsigned;
        iops.put_payload_entry_blocking(3, e4).unwrap();

        let idx = get_next_payloadidx_to_watch(&iops).unwrap();

        assert_eq!(idx, expected_idx);
    }

    #[test]
    fn test_determine_payload_next_status() {
        // When both are unpublished
        let (commit_status, reveal_status) = (L1TxStatus::Unpublished, L1TxStatus::Unpublished);
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::Unpublished);

        // When both are Finalized
        let fin = L1TxStatus::Finalized { confirmations: 5 };
        let (commit_status, reveal_status) = (fin.clone(), fin);
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::Finalized);

        // When both are Confirmed
        let conf = L1TxStatus::Confirmed { confirmations: 5 };
        let (commit_status, reveal_status) = (conf.clone(), conf.clone());
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::Confirmed);

        // When both are Published
        let publ = L1TxStatus::Published;
        let (commit_status, reveal_status) = (publ.clone(), publ.clone());
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::Published);

        // When both have invalid
        let (commit_status, reveal_status) = (L1TxStatus::InvalidInputs, L1TxStatus::InvalidInputs);
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::NeedsResign);

        // When reveal has invalid inputs but commit is confirmed. I doubt this would happen in
        // practice for our case.
        // Then the payload status should be NeedsResign i.e. the payload should be signed again and
        // published.
        let (commit_status, reveal_status) = (conf.clone(), L1TxStatus::InvalidInputs);
        let next = determine_payload_next_status(&commit_status, &reveal_status);
        assert_eq!(next, L1BundleStatus::NeedsResign);
    }
}
