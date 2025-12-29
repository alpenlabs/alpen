//! Update submitter task implementation.

use std::sync::Arc;

use alpen_ee_common::{
    BatchId, BatchProver, BatchStatus, BatchStorage, ExecBlockStorage, OLFinalizedStatus,
    SequencerOLClient,
};
use eyre::Result;
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::update_submitter::update_builder::build_update_from_batch;

/// Main update submitter task.
///
/// This task monitors for two triggers:
/// 1. New batch ready notifications
/// 2. OL chain status updates
///
/// On either trigger, it queries the OL client for the current account state,
/// finds all batches in `ProofReady` state starting from the next expected
/// sequence number, and submits them in order.
pub async fn update_submitter_task<C, S, ES, P>(
    ol_client: Arc<C>,
    batch_storage: Arc<S>,
    exec_storage: Arc<ES>,
    prover: Arc<P>,
    mut batch_ready_rx: watch::Receiver<Option<BatchId>>,
    mut ol_status_rx: watch::Receiver<OLFinalizedStatus>,
) where
    C: SequencerOLClient,
    S: BatchStorage,
    ES: ExecBlockStorage,
    P: BatchProver,
{
    loop {
        let result = tokio::select! {
            // Branch 1: New batch ready notification
            changed = batch_ready_rx.changed() => {
                if changed.is_err() {
                    warn!("batch_ready_rx closed; exiting");
                    return;
                }
                process_ready_batches(ol_client.as_ref(), batch_storage.as_ref(), exec_storage.as_ref(), prover.as_ref()).await
            }
            // Branch 2: OL chain status update
            changed = ol_status_rx.changed() => {
                if changed.is_err() {
                    warn!("ol_status_rx closed; exiting");
                    return;
                }
                process_ready_batches(ol_client.as_ref(), batch_storage.as_ref(), exec_storage.as_ref(), prover.as_ref()).await
            }
        };

        if let Err(e) = result {
            error!(error = %e, "Update submitter error");
        }
    }
}

/// Process all ready batches starting from the next expected sequence number.
///
/// Queries the OL client for the current account state, then iterates through
/// batches in storage starting from the next expected sequence number. For each
/// batch in `ProofReady` state, it builds and submits an update.
async fn process_ready_batches(
    ol_client: &impl SequencerOLClient,
    batch_storage: &impl BatchStorage,
    exec_storage: &impl ExecBlockStorage,
    prover: &impl BatchProver,
) -> Result<()> {
    // Get latest account state from OL to determine next expected seq_no
    let account_state = ol_client.get_latest_account_state().await?;
    // seq_no is the last processed update, so next is seq_no + 1
    // For batch idx, we use the same value (batch idx == seq_no for now)
    let current_seq_no = *account_state.seq_no.inner();
    let next_batch_idx = current_seq_no.saturating_add(1);

    let mut batch_idx = next_batch_idx;

    loop {
        let Some((batch, status)) = batch_storage.get_batch_by_idx(batch_idx).await? else {
            // No more batches
            break;
        };

        // Only process ProofReady batches
        let BatchStatus::ProofReady { da: _, proof } = status else {
            // Batch not ready yet, stop processing (must be sent in order)
            break;
        };

        // Build and submit update
        let update = build_update_from_batch(&batch, &proof, exec_storage, prover).await?;
        ol_client.submit_update(update).await?;

        debug!(batch_idx, "Submitted update for batch");
        batch_idx += 1;
    }

    Ok(())
}
