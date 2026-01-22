//! Update submitter task implementation.

use std::{collections::HashMap, sync::Arc};

use alpen_ee_common::{
    BatchId, BatchProver, BatchStatus, BatchStorage, ExecBlockStorage, OLFinalizedStatus,
    SequencerOLClient,
};
use eyre::{eyre, Result};
use strata_snark_acct_types::SnarkAccountUpdate;
use tokio::sync::watch;
use tracing::{debug, error, warn};

use crate::update_submitter::update_builder::build_update_from_batch;

/// Maximum number of entries in the update cache.
const UPDATE_CACHE_MAX_SIZE: usize = 64;

/// Cache for built updates, keyed by BatchId.
/// Stores (batch_idx, update) to allow eviction based on sequence number.
struct UpdateCache {
    entries: HashMap<BatchId, (u64, SnarkAccountUpdate)>,
}

impl UpdateCache {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Get a cached update by BatchId.
    fn get(&self, batch_id: &BatchId) -> Option<&SnarkAccountUpdate> {
        self.entries.get(batch_id).map(|(_, update)| update)
    }

    /// Insert an update into the cache if there is room.
    /// If the cache is at max capacity, the entry is not inserted.
    fn insert(&mut self, batch_id: BatchId, batch_idx: u64, update: SnarkAccountUpdate) {
        if self.entries.len() < UPDATE_CACHE_MAX_SIZE {
            self.entries.insert(batch_id, (batch_idx, update));
        }
    }

    /// Evict entries for batches that have been accepted (batch_idx < current_seq_no).
    fn evict_accepted(&mut self, current_seq_no: u64) {
        self.entries.retain(|_, (idx, _)| *idx >= current_seq_no);
    }
}

/// Main update submitter task.
///
/// This task monitors for two triggers:
/// 1. New batch ready notifications
/// 2. OL chain status updates
///
/// On either trigger, it queries the OL client for the current account state, finds all batches in
/// `ProofReady` state starting from the next expected sequence number, and submits them in order.
/// Depends on OL to dedupe transactions already in mempool.
pub async fn create_update_submitter_task<C, S, ES, P>(
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
    let mut update_cache = UpdateCache::new();

    loop {
        let result = tokio::select! {
            // Branch 1: New batch ready notification
            changed = batch_ready_rx.changed() => {
                if changed.is_err() {
                    warn!("batch_ready_rx closed; exiting");
                    return;
                }
                process_ready_batches(ol_client.as_ref(), batch_storage.as_ref(), exec_storage.as_ref(), prover.as_ref(), &mut update_cache).await
            }
            // Branch 2: OL chain status update
            changed = ol_status_rx.changed() => {
                if changed.is_err() {
                    warn!("ol_status_rx closed; exiting");
                    return;
                }
                process_ready_batches(ol_client.as_ref(), batch_storage.as_ref(), exec_storage.as_ref(), prover.as_ref(), &mut update_cache).await
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
    update_cache: &mut UpdateCache,
) -> Result<()> {
    // Get latest account state from OL to determine next expected seq_no
    let account_state = ol_client.get_latest_account_state().await?;
    let next_sequence_no = *account_state.seq_no.inner();
    // NOTE: ensure batch 0 (genesis batch) is never sent in an update.
    let next_batch_idx = next_sequence_no
        .checked_add(1)
        .ok_or_else(|| eyre!("max sequence number exceeded"))?; // shouldn't happen

    // Evict cache entries for batches that have been accepted
    update_cache.evict_accepted(next_sequence_no);

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

        // Get update from cache or build it
        let batch_id = batch.id();
        let update = if let Some(cached) = update_cache.get(&batch_id) {
            cached.clone()
        } else {
            let update = build_update_from_batch(&batch, &proof, exec_storage, prover).await?;
            update_cache.insert(batch_id, batch_idx, update.clone());
            update
        };

        ol_client.submit_update(update).await?;

        debug!(batch_idx, "Submitted update for batch");
        batch_idx += 1;
    }

    Ok(())
}
