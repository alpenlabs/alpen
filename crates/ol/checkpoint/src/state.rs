//! Service state for OL checkpoint builder.

use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, TerminalHeaderComplement,
};
use strata_codec::encode_to_vec;
use strata_db_types::types::OLCheckpointEntry;
use strata_identifiers::{Epoch, OLBlockCommitment};
use strata_ol_chain_types_new::OLBlockHeader;
use strata_ol_da::{OLDaPayloadV1, StateDiff};
use strata_primitives::epoch::EpochCommitment;
use strata_service::ServiceState;
use tracing::{debug, info, warn};

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

        // Determine starting epoch index (last processed + 1, or 1 if none, skip genesis epoch)
        let start_epoch_index = self.last_processed_epoch_index.map(|e| e + 1).unwrap_or(1);

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
            .ok_or(CheckpointNotReady::EpochCommitment(epoch_index))?;

        // Get summary - must exist to proceed
        let summary = self
            .ctx
            .get_epoch_summary(commitment)?
            .ok_or(CheckpointNotReady::EpochSummary(commitment))?;

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

        info!(
            component = "ol_checkpoint",
            %epoch,
            l1_height = summary.new_l1().height(),
            l1_block = %summary.new_l1(),
            l2_commitment = %summary.terminal(),
            "stored OL checkpoint entry"
        );
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
    let l1_height = summary.new_l1().height();
    let l2_commitment = *summary.terminal();
    let new_tip = CheckpointTip::new(summary.epoch(), l1_height, l2_commitment);

    let state_bytes = compute_da(&commitment, ctx)?;
    let ol_logs = ctx.get_epoch_logs(&commitment)?;
    let terminal_header = ctx
        .get_terminal_block_header(&l2_commitment)?
        .ok_or(CheckpointNotReady::TerminalBlock(l2_commitment))?;
    assert_terminal_commitment_matches(&terminal_header, l2_commitment)?;

    // Extract the four header fields not derivable from L1 checkpoint data
    // (timestamp, parent_blkid, body_root, logs_root). Other header fields are
    // derivable: slot/blkid from new_tip, state_root from DA + manifests.
    let terminal_header_complement = TerminalHeaderComplement::from_full_header(&terminal_header);

    let sidecar = CheckpointSidecar::new(state_bytes, ol_logs, terminal_header_complement)?;
    let proof = ctx.get_proof(&commitment)?;

    Ok(CheckpointPayload::new(new_tip, sidecar, proof)?)
}

fn assert_terminal_commitment_matches(
    terminal_header: &OLBlockHeader,
    expected_terminal: OLBlockCommitment,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        terminal_header.slot() == expected_terminal.slot(),
        "terminal header slot mismatch: expected {}, got {}",
        expected_terminal.slot(),
        terminal_header.slot()
    );
    anyhow::ensure!(
        terminal_header.compute_blkid() == *expected_terminal.blkid(),
        "terminal header block id mismatch: expected {:?}, got {:?}",
        expected_terminal.blkid(),
        terminal_header.compute_blkid()
    );
    Ok(())
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
    // TODO: Replace with real DA payload generation from epoch execution.
    // This encodes an empty StateDiff so the proof guest can decode it without panic.
    warn!("compute_da: returning empty DA payload (real DA generation not yet implemented)");
    let payload = OLDaPayloadV1::new(StateDiff::default());
    encode_to_vec(&payload).map_err(|e| anyhow::anyhow!("failed to encode empty DA payload: {e}"))
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
        Buf64, Epoch, OLBlockCommitment,
        test_utils::{buf32_strategy, l1_block_commitment_strategy, ol_block_commitment_strategy},
    };
    use strata_ol_chain_types_new::{
        BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
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
                let tip = CheckpointTip::new(summary.epoch(), summary.new_l1().height(), *summary.terminal());
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
            terminal_slot in any::<u64>(),
            body_root in buf32_strategy(),
            logs_root in buf32_strategy(),
            prev_terminal in ol_block_commitment_strategy(),
            genesis_l1 in l1_block_commitment_strategy(),
            new_l1 in l1_block_commitment_strategy(),
            final_state in buf32_strategy(),
        ) {
            let backend = get_test_sled_backend();
            let storage = Arc::new(
                create_node_storage(backend, threadpool::ThreadPool::new(1)).expect("test storage"),
            );
            let checkpoint_mgr = storage.ol_checkpoint();
            let ol_block_mgr = storage.ol_block();

            let epoch: Epoch = 1;
            let terminal_header = OLBlockHeader::new(
                1_700_000_000,
                BlockFlags::zero(),
                terminal_slot,
                epoch,
                *prev_terminal.blkid(),
                body_root,
                final_state,
                logs_root,
            );
            let terminal_block = OLBlock::new(
                SignedOLBlockHeader::new(terminal_header.clone(), Buf64::zero()),
                OLBlockBody::new_common(
                    OLTxSegment::new(vec![])
                        .expect("empty tx segment construction is infallible"),
                ),
            );
            ol_block_mgr
                .put_block_data_blocking(terminal_block)
                .expect("insert terminal block");

            let terminal = terminal_header.compute_block_commitment();
            let genesis_summary =
                EpochSummary::new(0, prev_terminal, OLBlockCommitment::null(), genesis_l1, final_state);
            checkpoint_mgr
                .insert_epoch_summary_blocking(genesis_summary)
                .expect("insert genesis summary");
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
                .expect("get checkpoint")
                .expect("checkpoint should be stored");
            let sidecar_terminal_subset = stored.checkpoint.sidecar().terminal_header_complement();

            prop_assert_eq!(sidecar_terminal_subset.timestamp(), terminal_header.timestamp());
            prop_assert_eq!(*sidecar_terminal_subset.parent_blkid(), *terminal_header.parent_blkid());
            prop_assert_eq!(*sidecar_terminal_subset.body_root(), *terminal_header.body_root());
            prop_assert_eq!(*sidecar_terminal_subset.logs_root(), *terminal_header.logs_root());
        }
    }
}
