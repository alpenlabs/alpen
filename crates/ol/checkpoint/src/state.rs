//! Service state for OL checkpoint builder.

use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::{CheckpointPayload, CheckpointSidecar, CheckpointTip};
use strata_db_types::types::OLCheckpointEntry;
use strata_identifiers::Epoch;
use strata_primitives::epoch::EpochCommitment;
use strata_service::ServiceState;
use tracing::debug;

use crate::{context::CheckpointWorkerContext, errors::CheckpointNotReady};

/// Service state for OL checkpoint builder.
///
/// Generic over the context to allow testing with mock implementations.
pub(crate) struct OLCheckpointServiceState<C: CheckpointWorkerContext> {
    ctx: C,
    initialized: bool,
    last_processed_epoch: Option<Epoch>,
    last_processed_epoch_index: Option<u64>,
}

impl<C: CheckpointWorkerContext> OLCheckpointServiceState<C> {
    /// Create a new state with the given context.
    pub(crate) fn new(ctx: C) -> Self {
        Self {
            ctx,
            initialized: false,
            last_processed_epoch: None,
            last_processed_epoch_index: None,
        }
    }

    pub(crate) fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub(crate) fn last_processed_epoch(&self) -> Option<Epoch> {
        self.last_processed_epoch
    }

    pub(crate) fn initialize(&mut self) {
        self.init_cursor_from_db();
        self.initialized = true;
    }

    /// Handles a completed epoch, catching up from last checkpoint to latest summary.
    ///
    /// The `target` commitment identifies the epoch that was completed. We process
    /// all pending epochs up to and including the latest summarized epoch.
    pub(crate) fn handle_complete_epoch(&mut self, target: EpochCommitment) -> anyhow::Result<()> {
        anyhow::ensure!(self.initialized, "worker not initialized");

        let Some(target_epoch_index) = self.ctx.get_last_summarized_epoch()? else {
            return Ok(());
        };

        // Determine starting epoch index (last processed + 1, or 0 if none)
        let start_epoch_index = self.last_processed_epoch_index.map(|e| e + 1).unwrap_or(0);

        // Process all epochs from start to target (inclusive)
        for epoch_index in start_epoch_index..=target_epoch_index {
            self.process_epoch(epoch_index)?;
        }

        // Sanity check: verify we processed up to at least the target epoch
        if let Some(last_epoch) = self.last_processed_epoch
            && last_epoch < target.epoch()
        {
            debug!(
                last_processed = last_epoch,
                target_epoch = target.epoch(),
                "processed epochs but not yet caught up to target"
            );
        }

        Ok(())
    }

    /// Process a single epoch, building checkpoint if summary exists.
    ///
    /// Returns error if the epoch index cannot be processed (missing data).
    /// Checkpoints must be built sequentially, so caller should stop on error.
    fn process_epoch(&mut self, epoch_index: u64) -> anyhow::Result<()> {
        // Get canonical commitment for this epoch index - must exist to proceed
        let commitment = self
            .ctx
            .get_canonical_epoch_commitment_at(epoch_index)?
            .ok_or(CheckpointNotReady::MissingEpochCommitment(epoch_index))?;

        // Get summary - must exist to proceed
        let summary = self
            .ctx
            .get_epoch_summary(commitment)?
            .ok_or(CheckpointNotReady::MissingEpochSummary(commitment))?;

        let epoch = summary.epoch();

        // Skip if already checkpointed
        if self.ctx.get_checkpoint(epoch)?.is_some() {
            self.last_processed_epoch = Some(epoch);
            self.last_processed_epoch_index = Some(epoch_index);
            return Ok(());
        }

        let payload = build_checkpoint_payload(commitment, &summary, &self.ctx)?;
        let entry = OLCheckpointEntry::new_unsigned(payload);
        self.ctx.put_checkpoint(epoch, entry)?;

        debug!(epoch, "stored OL checkpoint entry");
        self.last_processed_epoch = Some(epoch);
        self.last_processed_epoch_index = Some(epoch_index);

        Ok(())
    }

    fn init_cursor_from_db(&mut self) {
        let Ok(Some(last_checkpoint_epoch)) = self.ctx.get_last_checkpoint_epoch() else {
            return;
        };

        let Ok(Some(last_summarized_index)) = self.ctx.get_last_summarized_epoch() else {
            return;
        };

        for epoch_index in (0..=last_summarized_index).rev() {
            let Ok(Some(commitment)) = self.ctx.get_canonical_epoch_commitment_at(epoch_index)
            else {
                continue;
            };
            let Ok(Some(summary)) = self.ctx.get_epoch_summary(commitment) else {
                continue;
            };

            if summary.epoch() == last_checkpoint_epoch {
                self.last_processed_epoch = Some(last_checkpoint_epoch);
                self.last_processed_epoch_index = Some(epoch_index);
                break;
            }
        }
    }
}

impl<C: CheckpointWorkerContext> ServiceState for OLCheckpointServiceState<C> {
    fn name(&self) -> &str {
        "ol_checkpoint"
    }
}

fn build_checkpoint_payload<C: CheckpointWorkerContext>(
    commitment: EpochCommitment,
    summary: &EpochSummary,
    ctx: &C,
) -> anyhow::Result<CheckpointPayload> {
    let l1_height = summary.new_l1().height_u32();
    let l2_commitment = *summary.terminal();
    let new_tip = CheckpointTip::new(summary.epoch(), l1_height, l2_commitment);

    let state_bytes = compute_da(&commitment, ctx)?;
    let ol_logs = ctx.get_epoch_logs(&commitment)?;

    let sidecar = CheckpointSidecar::new(state_bytes, ol_logs)?;
    let proof = ctx.get_proof(&commitment)?;

    Ok(CheckpointPayload::new(new_tip, sidecar, proof)?)
}

/// Computes the DA state diff for the epoch.
///
/// DA generation is the checkpoint service's responsibility, not a storage read.
/// When fully implemented, this will read OL state changes from the context and
/// assemble them using DA framework primitives.
fn compute_da<C: CheckpointWorkerContext>(
    _commitment: &EpochCommitment,
    _ctx: &C,
) -> anyhow::Result<Vec<u8>> {
    // V1: empty DA bytes
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;
    use strata_checkpoint_types::EpochSummary;
    use strata_checkpoint_types_ssz::{
        CheckpointPayload, CheckpointTip, test_utils::checkpoint_sidecar_strategy,
    };
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{
        Epoch, OLBlockCommitment,
        test_utils::{buf32_strategy, l1_block_commitment_strategy, ol_block_commitment_strategy},
    };
    use strata_storage::create_node_storage;

    use super::OLCheckpointServiceState;
    use crate::context::CheckpointWorkerContextImpl;

    proptest! {
        #[test]
        fn init_cursor_from_db_uses_last_checkpoint_epoch(
            len in 1usize..=5,
            terminals in prop::collection::vec(ol_block_commitment_strategy(), 1..=5),
            l1s in prop::collection::vec(l1_block_commitment_strategy(), 1..=5),
            finals in prop::collection::vec(buf32_strategy(), 1..=5),
            sidecars in prop::collection::vec(checkpoint_sidecar_strategy(), 1..=5),
            last_checkpoint in 0usize..=4,
        ) {
            let len = len.min(terminals.len())
                .min(l1s.len())
                .min(finals.len())
                .min(sidecars.len());
            prop_assume!(len > 0);
            let last_checkpoint = last_checkpoint.min(len.saturating_sub(1));

            let backend = get_test_sled_backend();
            let storage = Arc::new(
                create_node_storage(backend, threadpool::ThreadPool::new(1))
                    .expect("test storage"),
            );
            let checkpoint_mgr = storage.ol_checkpoint();

            let mut prev_terminal = OLBlockCommitment::null();
            let mut summaries = Vec::with_capacity(len);
            for i in 0..len {
                let epoch = i as Epoch;
                let terminal = terminals[i];
                let new_l1 = l1s[i];
                let summary = EpochSummary::new(
                    epoch,
                    terminal,
                    prev_terminal,
                    new_l1,
                    finals[i],
                );
                prev_terminal = terminal;
                checkpoint_mgr
                    .insert_epoch_summary_blocking(summary)
                    .expect("insert summary");
                summaries.push(summary);
            }

            for i in 0..=last_checkpoint {
                let summary = &summaries[i];
                let tip = CheckpointTip::new(summary.epoch(), summary.new_l1().height_u32(), *summary.terminal());
                let payload = CheckpointPayload::new(tip, sidecars[i].clone(), Vec::new())
                    .expect("payload");
                checkpoint_mgr
                    .put_checkpoint_blocking(
                        summary.epoch(),
                        super::OLCheckpointEntry::new_unsigned(payload),
                    )
                    .expect("put checkpoint");
            }

            let ctx = CheckpointWorkerContextImpl::new(storage);
            let mut state = OLCheckpointServiceState::new(ctx);
            state.initialize();

            assert_eq!(state.last_processed_epoch(), Some(last_checkpoint as Epoch));
            assert_eq!(state.last_processed_epoch_index, Some(last_checkpoint as u64));
        }
    }

    proptest! {
        #[test]
        fn builds_checkpoint_from_epoch_summary(
            terminal in ol_block_commitment_strategy(),
            prev_terminal in ol_block_commitment_strategy(),
            new_l1 in l1_block_commitment_strategy(),
            final_state in buf32_strategy(),
        ) {
            let backend = get_test_sled_backend();
            let storage = Arc::new(
                create_node_storage(backend, threadpool::ThreadPool::new(1)).expect("test storage"),
            );
            let checkpoint_mgr = storage.ol_checkpoint();

            let epoch: Epoch = 0;
            let summary = EpochSummary::new(epoch, terminal, prev_terminal, new_l1, final_state);
            let commitment = summary.get_epoch_commitment();
            checkpoint_mgr
                .insert_epoch_summary_blocking(summary)
                .expect("insert summary");

            let ctx = CheckpointWorkerContextImpl::new(Arc::clone(&storage));
            let mut state = OLCheckpointServiceState::new(ctx);
            state.initialize();

            state
                .handle_complete_epoch(commitment)
                .expect("build checkpoint");

            let stored = checkpoint_mgr
                .get_checkpoint_blocking(epoch)
                .expect("get checkpoint");
            prop_assert!(stored.is_some());
        }
    }
}
