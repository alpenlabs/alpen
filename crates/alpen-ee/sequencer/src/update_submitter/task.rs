//! Update submitter task implementation.

use std::{collections::HashMap, sync::Arc, time::Duration};

use alpen_ee_common::{
    BatchId, BatchProver, BatchStatus, BatchStorage, ExecBlockStorage, OLFinalizedStatus,
    SequencerOLClient,
};
use eyre::{eyre, Result};
use strata_snark_acct_types::SnarkAccountUpdate;
use tokio::{sync::watch, time};
use tracing::{debug, error, info, warn};

use crate::update_submitter::update_builder::build_update_from_batch;

/// Maximum number of entries in the update cache.
const DEFAULT_UPDATE_CACHE_MAX_SIZE: usize = 64;
/// Polling interval to process batches regardless of events.
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(60);
/// Minimum wait before retrying the same submitted update while OL state is unchanged.
const DEFAULT_RESUBMIT_BACKOFF: Duration = DEFAULT_POLL_INTERVAL;

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
        if self.entries.len() < DEFAULT_UPDATE_CACHE_MAX_SIZE {
            self.entries.insert(batch_id, (batch_idx, update));
        }
    }

    /// Evict entries for batches that have been accepted (batch_idx < current_seq_no).
    fn evict_accepted(&mut self, current_seq_no: u64) {
        self.entries.retain(|_, (idx, _)| *idx >= current_seq_no);
    }
}

/// Tracks the head batch most recently submitted while waiting for OL state to advance.
struct PendingSubmission {
    /// Batch ID of the most recently submitted batch.
    batch_id: BatchId,

    /// Sequence number of the most recently submitted batch.
    seq_no: u64,

    /// Timestamp when the most recently submitted batch was submitted.
    submitted_at: time::Instant,
}

/// Tracks the state of the update submitter.
struct SubmitState {
    /// Most recently submitted batch while waiting for OL state to advance.
    pending: Option<PendingSubmission>,
}

impl SubmitState {
    /// Creates a new submit state with no pending submission.
    fn new() -> Self {
        Self { pending: None }
    }

    /// Clears pending state once OL account sequence number changes.
    fn clear_if_ol_advanced(&mut self, current_seq_no: u64) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.seq_no != current_seq_no)
        {
            self.pending = None;
        }
    }

    /// Returns true when the same head batch was already submitted recently.
    fn should_skip_resubmission(&self, batch_id: BatchId, seq_no: u64, now: time::Instant) -> bool {
        self.pending.as_ref().is_some_and(|pending| {
            pending.batch_id == batch_id
                && pending.seq_no == seq_no
                && now.duration_since(pending.submitted_at) < DEFAULT_RESUBMIT_BACKOFF
        })
    }

    /// Records a new submission.
    fn record_submission(&mut self, batch_id: BatchId, seq_no: u64, submitted_at: time::Instant) {
        self.pending = Some(PendingSubmission {
            batch_id,
            seq_no,
            submitted_at,
        });
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
    let mut submit_state = SubmitState::new();

    // run a first pass on start without waiting for any events
    if let Err(e) = process_ready_batches(
        ol_client.as_ref(),
        batch_storage.as_ref(),
        exec_storage.as_ref(),
        prover.as_ref(),
        &mut update_cache,
        &mut submit_state,
    )
    .await
    {
        error!(error = %e, "Update submitter error");
    }

    // afterwards, process ready batches at fixed intervals, and after ol or batch changes
    let mut poll_interval = time::interval(DEFAULT_POLL_INTERVAL);
    loop {
        tokio::select! {
            // Branch 1: New batch ready notification
            changed = batch_ready_rx.changed() => {
                if changed.is_err() {
                    warn!("batch_ready_rx closed; exiting");
                    return;
                }
            }
            // Branch 2: OL chain status update
            changed = ol_status_rx.changed() => {
                if changed.is_err() {
                    warn!("ol_status_rx closed; exiting");
                    return;
                }
            }
            // Branch 3: Poll interval tick
            _ = poll_interval.tick() => { }
        };

        if let Err(e) = process_ready_batches(
            ol_client.as_ref(),
            batch_storage.as_ref(),
            exec_storage.as_ref(),
            prover.as_ref(),
            &mut update_cache,
            &mut submit_state,
        )
        .await
        {
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
    submit_state: &mut SubmitState,
) -> Result<()> {
    // Get latest account state from OL to determine next expected seq_no
    let account_state = ol_client.get_latest_account_state().await?;
    debug!(?account_state, "Latest account state");
    let next_sequence_no = *account_state.seq_no.inner();
    // NOTE: ensure batch 0 (genesis batch) is never sent in an update.
    let next_batch_idx = next_sequence_no
        .checked_add(1)
        .ok_or_else(|| eyre!("max sequence number exceeded"))?; // shouldn't happen

    // Evict cache entries for batches that have been accepted
    update_cache.evict_accepted(next_sequence_no);
    submit_state.clear_if_ol_advanced(next_sequence_no);

    let mut batch_idx = next_batch_idx;

    loop {
        let Some((batch, status)) = batch_storage.get_batch_by_idx(batch_idx).await? else {
            // No more batches
            debug!(%batch_idx, "Got no batch. breaking");
            break;
        };
        debug!(?batch, ?status, "Got batch");

        // Only process ProofReady batches
        let BatchStatus::ProofReady { da, proof } = status else {
            // Batch not ready yet, stop processing (must be sent in order)
            debug!(%batch_idx, "Batch not ready");
            break;
        };

        // Get update from cache or build it
        let batch_id = batch.id();
        let update = if let Some(cached) = update_cache.get(&batch_id) {
            cached.clone()
        } else {
            let update =
                build_update_from_batch(&batch, &da, &proof, ol_client, exec_storage, prover)
                    .await?;
            update_cache.insert(batch_id, batch_idx, update.clone());
            update
        };

        let seq_no = update.operation().seq_no();
        let now = time::Instant::now();

        // Check if the same batch was already submitted recently
        if submit_state.should_skip_resubmission(batch_id, seq_no, now) {
            debug!(
                %batch_idx,
                ?batch_id,
                %seq_no,
                "Skipping resubmission while waiting for OL account state to advance"
            );
            break;
        }

        let l1_ref_count = update.operation().ledger_refs().l1_header_refs().len();
        let txid = ol_client.submit_update(update).await?;
        submit_state.record_submission(batch_id, seq_no, now);

        info!(
            component = "alpen_ee_update_submitter",
            %batch_idx,
            ?batch_id,
            %txid,
            seq_no,
            proof_id = %proof,
            prev_block = %batch.prev_block(),
            last_block = %batch.last_block(),
            l1_ref_count,
            "Submitted update for batch"
        );

        batch_idx += 1;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use alpen_ee_common::{
        InMemoryStorage, MockBatchProver, MockExecBlockStorage, MockSequencerOLClient,
        OLAccountStateView,
    };
    use mockall::Sequence;
    use strata_identifiers::OLTxId;
    use strata_snark_acct_types::{
        LedgerRefs, ProofState, Seqno, UpdateOperationData, UpdateOutputs,
    };
    use tokio::time;

    use super::*;
    use crate::batch_lifecycle::test_utils::{
        fill_storage, make_batch, test_hash, TestBatchStatus,
    };

    fn make_account_state(seq_no: u64) -> OLAccountStateView {
        OLAccountStateView {
            seq_no: Seqno::new(seq_no),
            proof_state: ProofState::new(test_hash(99), 0),
        }
    }

    fn make_cached_update(seq_no: u64) -> SnarkAccountUpdate {
        let operation = UpdateOperationData::new(
            seq_no,
            ProofState::new(test_hash(42), 0),
            vec![],
            LedgerRefs::new_empty(),
            UpdateOutputs::new_empty(),
            vec![],
        );

        SnarkAccountUpdate::new(operation, vec![0u8; 32])
    }

    #[tokio::test]
    async fn submits_head_batch_once_while_ol_seq_no_is_unchanged() {
        let storage = InMemoryStorage::new_empty();
        let batches = fill_storage(&storage, &[TestBatchStatus::ProofReady]).await;
        let batch = batches[1].clone();
        let batch_id = batch.id();

        let mut ol_client = MockSequencerOLClient::new();
        ol_client
            .expect_get_latest_account_state()
            .times(2)
            .returning(|| Ok(make_account_state(0)));
        ol_client
            .expect_submit_update()
            .times(1)
            .returning(|_| Ok(OLTxId::default()));

        let exec_storage = MockExecBlockStorage::new();
        let prover = MockBatchProver::new();
        let mut update_cache = UpdateCache::new();
        let mut submit_state = SubmitState::new();
        update_cache.insert(batch_id, batch.idx(), make_cached_update(0));

        process_ready_batches(
            &ol_client,
            &storage,
            &exec_storage,
            &prover,
            &mut update_cache,
            &mut submit_state,
        )
        .await
        .unwrap();

        process_ready_batches(
            &ol_client,
            &storage,
            &exec_storage,
            &prover,
            &mut update_cache,
            &mut submit_state,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn resubmits_after_backoff_if_ol_seq_no_is_still_unchanged() {
        let storage = InMemoryStorage::new_empty();
        let batches = fill_storage(&storage, &[TestBatchStatus::ProofReady]).await;
        let batch = batches[1].clone();

        let mut ol_client = MockSequencerOLClient::new();
        ol_client
            .expect_get_latest_account_state()
            .times(2)
            .returning(|| Ok(make_account_state(0)));
        ol_client
            .expect_submit_update()
            .times(2)
            .returning(|_| Ok(OLTxId::default()));

        let exec_storage = MockExecBlockStorage::new();
        let prover = MockBatchProver::new();
        let mut update_cache = UpdateCache::new();
        let mut submit_state = SubmitState::new();
        update_cache.insert(batch.id(), batch.idx(), make_cached_update(0));

        process_ready_batches(
            &ol_client,
            &storage,
            &exec_storage,
            &prover,
            &mut update_cache,
            &mut submit_state,
        )
        .await
        .unwrap();

        submit_state.pending.as_mut().unwrap().submitted_at =
            time::Instant::now() - DEFAULT_RESUBMIT_BACKOFF;

        process_ready_batches(
            &ol_client,
            &storage,
            &exec_storage,
            &prover,
            &mut update_cache,
            &mut submit_state,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn submits_next_batch_immediately_after_ol_seq_no_advances() {
        let storage = InMemoryStorage::new_empty();
        let batches = fill_storage(
            &storage,
            &[TestBatchStatus::ProofReady, TestBatchStatus::ProofReady],
        )
        .await;
        let first_batch = batches[1].clone();
        let second_batch = batches[2].clone();

        let mut ol_client = MockSequencerOLClient::new();
        let mut seq = Sequence::new();
        ol_client
            .expect_get_latest_account_state()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(make_account_state(0)));
        ol_client
            .expect_get_latest_account_state()
            .times(1)
            .in_sequence(&mut seq)
            .returning(|| Ok(make_account_state(1)));

        let mut submits = Sequence::new();
        ol_client
            .expect_submit_update()
            .times(1)
            .withf(|update| update.operation().seq_no() == 0)
            .in_sequence(&mut submits)
            .returning(|_| Ok(OLTxId::default()));
        ol_client
            .expect_submit_update()
            .times(1)
            .withf(|update| update.operation().seq_no() == 1)
            .in_sequence(&mut submits)
            .returning(|_| Ok(OLTxId::default()));

        let exec_storage = MockExecBlockStorage::new();
        let prover = MockBatchProver::new();
        let mut update_cache = UpdateCache::new();
        let mut submit_state = SubmitState::new();
        update_cache.insert(first_batch.id(), first_batch.idx(), make_cached_update(0));
        update_cache.insert(second_batch.id(), second_batch.idx(), make_cached_update(1));

        process_ready_batches(
            &ol_client,
            &storage,
            &exec_storage,
            &prover,
            &mut update_cache,
            &mut submit_state,
        )
        .await
        .unwrap();

        process_ready_batches(
            &ol_client,
            &storage,
            &exec_storage,
            &prover,
            &mut update_cache,
            &mut submit_state,
        )
        .await
        .unwrap();
    }

    #[test]
    fn clears_pending_submission_when_ol_seq_no_changes() {
        let batch = make_batch(1, 1, 2);
        let mut submit_state = SubmitState::new();
        submit_state.record_submission(batch.id(), 0, time::Instant::now());

        submit_state.clear_if_ol_advanced(1);

        assert!(submit_state.pending.is_none());
    }
}
