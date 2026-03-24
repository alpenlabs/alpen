//! Service state for OL checkpoint builder.

use strata_checkpoint_types::EpochSummary;
use strata_checkpoint_types_ssz::{
    CheckpointPayload, CheckpointSidecar, CheckpointTip, TerminalHeaderComplement,
};
use strata_db_types::types::OLCheckpointEntry;
use strata_identifiers::Epoch;
use strata_primitives::epoch::EpochCommitment;
use strata_service::ServiceState;
use tracing::{debug, info};

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

    let (state_bytes, ol_logs) = ctx.fetch_da_for_epoch(summary)?;

    let terminal_header = ctx
        .get_block_header(summary.terminal())?
        .ok_or_else(|| anyhow::anyhow!("missing terminal block for epoch summary {:?}", summary))?;
    let terminal_header_complement = TerminalHeaderComplement::from_full_header(&terminal_header);

    let sidecar = CheckpointSidecar::new(state_bytes, ol_logs, terminal_header_complement)?;
    let proof = ctx.get_proof(&commitment)?;

    Ok(CheckpointPayload::new(new_tip, sidecar, proof)?)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proptest::prelude::*;
    use strata_checkpoint_types::EpochSummary;
    use strata_checkpoint_types_ssz::{
        CheckpointPayload, CheckpointTip,
        test_utils::{checkpoint_sidecar_strategy, ol_logs_strategy, state_diff_strategy},
    };
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::types::OLCheckpointEntry;
    use strata_identifiers::{
        Buf64, Epoch, OLBlockCommitment,
        test_utils::{buf32_strategy, l1_block_commitment_strategy, ol_block_commitment_strategy},
    };
    use strata_ol_chain_types_new::{
        BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLBlockId, OLLog, OLTxSegment,
        SignedOLBlockHeader,
    };
    use strata_ol_state_types::OLState;
    use strata_primitives::epoch::EpochCommitment;
    use strata_storage::create_node_storage;

    use super::OLCheckpointServiceState;
    use crate::context::{CheckpointWorkerContext, CheckpointWorkerContextImpl, StateDiffRaw};

    /// Test context that delegates everything to the real impl but stubs out
    /// `fetch_da_for_epoch` with provided DA data. This avoids needing a full
    /// replay chain (prev terminal block, OL state, etc.) in structural tests.
    struct TestCheckpointContext {
        inner: CheckpointWorkerContextImpl,
        stub_state_diff: StateDiffRaw,
        stub_ol_logs: Vec<OLLog>,
    }

    impl TestCheckpointContext {
        fn new(
            storage: Arc<strata_storage::NodeStorage>,
            stub_state_diff: StateDiffRaw,
            stub_ol_logs: Vec<OLLog>,
        ) -> Self {
            Self {
                inner: CheckpointWorkerContextImpl::new(storage),
                stub_state_diff,
                stub_ol_logs,
            }
        }
    }

    impl CheckpointWorkerContext for TestCheckpointContext {
        fn get_last_summarized_epoch(&self) -> anyhow::Result<Option<u64>> {
            self.inner.get_last_summarized_epoch()
        }

        fn get_canonical_epoch_commitment_at(
            &self,
            index: u64,
        ) -> anyhow::Result<Option<EpochCommitment>> {
            self.inner.get_canonical_epoch_commitment_at(index)
        }

        fn get_epoch_summary(
            &self,
            commitment: EpochCommitment,
        ) -> anyhow::Result<Option<EpochSummary>> {
            self.inner.get_epoch_summary(commitment)
        }

        fn get_checkpoint(&self, epoch: Epoch) -> anyhow::Result<Option<OLCheckpointEntry>> {
            self.inner.get_checkpoint(epoch)
        }

        fn get_last_checkpoint_epoch(&self) -> anyhow::Result<Option<Epoch>> {
            self.inner.get_last_checkpoint_epoch()
        }

        fn put_checkpoint(&self, epoch: Epoch, entry: OLCheckpointEntry) -> anyhow::Result<()> {
            self.inner.put_checkpoint(epoch, entry)
        }

        fn get_proof(&self, epoch: &EpochCommitment) -> anyhow::Result<Vec<u8>> {
            self.inner.get_proof(epoch)
        }

        fn get_block_header(
            &self,
            blkid: &OLBlockCommitment,
        ) -> anyhow::Result<Option<OLBlockHeader>> {
            self.inner.get_block_header(blkid)
        }

        fn get_block(&self, id: &OLBlockId) -> anyhow::Result<Option<OLBlock>> {
            self.inner.get_block(id)
        }

        fn get_ol_state(&self, commitment: &OLBlockCommitment) -> anyhow::Result<Option<OLState>> {
            self.inner.get_ol_state(commitment)
        }

        fn fetch_da_for_epoch(
            &self,
            _summary: &EpochSummary,
        ) -> anyhow::Result<(StateDiffRaw, Vec<OLLog>)> {
            Ok((self.stub_state_diff.clone(), self.stub_ol_logs.clone()))
        }
    }

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
            prev_terminal in ol_block_commitment_strategy(),
            slot_offset in 1..u64::MAX,
            body_root in buf32_strategy(),
            logs_root in buf32_strategy(),
            genesis_l1 in l1_block_commitment_strategy(),
            new_l1 in l1_block_commitment_strategy(),
            final_state in buf32_strategy(),
            state_diff in state_diff_strategy(),
            ol_logs in ol_logs_strategy(),
        ) {
            let backend = get_test_sled_backend();
            let storage = Arc::new(
                create_node_storage(backend, threadpool::ThreadPool::new(1)).expect("test storage"),
            );
            let checkpoint_mgr = storage.ol_checkpoint();
            let ol_block_mgr = storage.ol_block();

            let epoch: Epoch = 1;
            let terminal_slot = prev_terminal.slot().saturating_add(slot_offset);
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

            let ctx = TestCheckpointContext::new(Arc::clone(&storage), state_diff, ol_logs);
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
