//! Checkpoint log processing logic.

use std::sync::Arc;

use anyhow::Context;
use bitcoin::hashes::Hash;
use strata_asm_common::{AsmLogEntry, Subprotocol, VerifiedAuxData};
use strata_asm_logs::{CheckpointTipUpdate, constants::CHECKPOINT_TIP_UPDATE_LOG_TYPE};
use strata_asm_proto_checkpoint::{CheckpointState, CheckpointSubprotocol};
use strata_checkpoint_types::BatchInfo;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
use strata_identifiers::Epoch;
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;
use tracing::*;

use crate::{
    checkpoint_extract::extract_matching_checkpoint, context::CsmWorkerContext,
    state::CsmWorkerState,
};

impl<C: CsmWorkerContext> CsmWorkerState<C> {
    pub(crate) fn process_log(
        &mut self,
        log: &AsmLogEntry,
        asm_block: &L1BlockCommitment,
    ) -> anyhow::Result<()> {
        match log.ty() {
            Some(CHECKPOINT_TIP_UPDATE_LOG_TYPE) => {
                let tip_upd = log.try_into_log().map_err(|e| {
                    anyhow::anyhow!("Failed to deserialize CheckpointTipUpdate: {}", e)
                })?;

                return self.process_checkpoint_tip_log(&tip_upd, asm_block);
            }
            Some(log_type) => {
                debug!(log_type, "log type not processed by CSM");
            }
            None => {
                warn!("logs without a type ID?");
            }
        }
        Ok(())
    }

    /// Process a checkpoint tip update log from the checkpoint subprotocol.
    fn process_checkpoint_tip_log(
        &mut self,
        checkpoint_tip_update: &CheckpointTipUpdate,
        asm_block: &L1BlockCommitment,
    ) -> anyhow::Result<()> {
        let tip = checkpoint_tip_update.tip();
        let epoch = tip.epoch;
        let _span = info_span!("process_checkpoint_tip_log", %epoch).entered();

        info!(
            %asm_block,
            l1_height = tip.l1_height(),
            l2_slot = tip.l2_commitment().slot(),
            "CSM is processing checkpoint tip update from ASM log"
        );

        let l1_height = tip.l1_height();
        if l1_height != asm_block.height() {
            debug!(
                tip_l1_height = l1_height,
                asm_block_height = asm_block.height(),
                "Checkpoint tip L1 height differs from current ASM block height; using ASM block commitment"
            );
        }

        let observation = self.mark_ol_checkpoint_l1_observed(checkpoint_tip_update, asm_block)?;
        // Tip logs do not contain full batch transition details.
        // CSM only needs epoch progression for finalized-epoch signaling, so we
        // synthesize a minimal checkpoint view from the tip.
        // TODO(STR-2438): Remove this synthetic mapping once CSM persists/consumes
        // these fields directly without legacy L1Checkpoint shape coupling.
        let synthetic_checkpoint =
            checkpoint_from_tip_update(checkpoint_tip_update, asm_block, observation);
        self.apply_checkpoint_to_client_state(synthetic_checkpoint, epoch);

        self.staged.last_processed_epoch = Some(epoch);
        Ok(())
    }

    /// Updates the in-memory client state.
    ///
    /// This does NOT persist anything.
    fn apply_checkpoint_to_client_state(&mut self, new_checkpoint: L1Checkpoint, epoch: Epoch) {
        let cur_state = self.staged.cur_state.as_ref();

        // Determine if this checkpoint should be the last finalized or just recent.

        // TODO(STR-2438): This comes from the legacy design currently and will be
        // simplified in the future.
        // Currently, `last_finalized` is the buried checkpoint and recent and the last be observed
        // (the checkpoint that makes the the finalized one to be buried).

        // TODO(STR-2438): it's better to store `L1Checkpoint` separately, move the
        // logic of "recent/finalized" to the DbManager (that can actually fetches
        // actual persisted data and doesn't rely on the current state).
        let (last_finalized, recent) = match cur_state.get_last_checkpoint() {
            Some(existing) => {
                // If the new checkpoint is for a later epoch, it becomes recent
                if epoch > existing.batch_info.epoch() {
                    (Some(existing.clone()), Some(new_checkpoint))
                } else {
                    // Otherwise keep existing
                    (Some(existing.clone()), None)
                }
            }
            None => {
                // New checkpoint is the first checkpoint, and it is marked recent
                (None, Some(new_checkpoint))
            }
        };

        self.staged.cur_state = Arc::new(ClientState::new(last_finalized, recent));
    }

    /// Persists the client state for `asm_block` and advances the in-memory `last_asm_block`.
    ///
    /// Called after every log of the asm block processed without error.
    fn commit_block(&mut self, asm_block: L1BlockCommitment) -> anyhow::Result<()> {
        let next_state = self.staged.cur_state.as_ref().clone();
        self.ctx.put_client_state_update(
            &asm_block,
            ClientUpdateOutput::new(next_state.clone(), vec![]),
        )?;
        self.last_asm_block = Some(asm_block);
        // Publish the client state to status channel.
        self.ctx.publish_client_state(next_state, asm_block);
        Ok(())
    }

    /// Processes every log of a single ASM block and commits it as one unit.
    ///
    /// All staged state is snapshotted up front and restored on any failure
    /// including a `commit_block` failure.
    fn process_asm_logs(
        &mut self,
        asm_block: L1BlockCommitment,
        logs: &[AsmLogEntry],
    ) -> anyhow::Result<()> {
        let staged_snapshot = self.staged.clone();

        for log in logs {
            if let Err(e) = self.process_log(log, &asm_block) {
                self.staged = staged_snapshot;
                return Err(e).with_context(|| format!("processing ASM log for block {asm_block}"));
            }
        }

        if let Err(e) = self.commit_block(asm_block) {
            self.staged = staged_snapshot;
            return Err(e).with_context(|| format!("committing CSM block {asm_block}"));
        }

        Ok(())
    }

    /// Processes `asm_block` and its logs, first replaying any ASM blocks skipped
    /// between the last committed block and `asm_block`.
    ///
    ///  On error the cursor stays at the last contiguous block, so restarts resume safely.
    pub(crate) fn process_asm_block(
        &mut self,
        asm_block: L1BlockCommitment,
        logs: &[AsmLogEntry],
    ) -> anyhow::Result<()> {
        let last = self
            .last_asm_block
            .ok_or_else(|| anyhow::anyhow!("CSM has no last committed ASM block"))?;
        let last_height = last.height();
        let target_height = asm_block.height();

        // Exact duplicate — ASM redelivered the same status, nothing to do.
        if asm_block == last {
            debug!(%asm_block, "ASM status block matches last committed; skipping");
            return Ok(());
        }

        // TODO(STR-3466): Strictly behind — either a stale message or a deeper reorg.
        if target_height < last_height {
            warn!(
                %asm_block,
                last_height,
                "ASM block strictly behind last committed; skipping (possible deeper reorg)"
            );
            return Ok(());
        }

        // Same height, different blkid — the chain reorged out our last tip.
        // TODO(STR-3466): with this ticket the two conditionals can be collapsed into one `<=`
        // check.
        if target_height == last_height {
            warn!(
                %asm_block,
                last_blkid = ?last.blkid(),
                "same-height ASM block with different blkid; processing as reorged tip"
            );
            // Directly process the logs in one block reorg.
            return self.process_asm_logs(asm_block, logs);
        }

        for height in (last_height + 1)..=target_height {
            let (block, block_logs) = if height == target_height {
                (asm_block, logs.to_vec())
            } else {
                // fetch from db.
                let gap_block = self
                    .ctx
                    .get_canonical_l1_block(height)
                    .with_context(|| format!("resolving gap L1 block at height {height}"))?;
                let gap_state = self
                    .ctx
                    .get_asm_state(&gap_block)
                    .with_context(|| format!("fetching ASM state for gap block {gap_block}"))?;
                info!(%gap_block, "replaying ASM block skipped by status channel");
                (gap_block, gap_state.logs().clone())
            };
            self.process_asm_logs(block, &block_logs)?;
        }
        Ok(())
    }

    fn mark_ol_checkpoint_l1_observed(
        &mut self,
        checkpoint_tip_update: &CheckpointTipUpdate,
        asm_block: &L1BlockCommitment,
    ) -> anyhow::Result<CheckpointL1Ref> {
        let tip = checkpoint_tip_update.tip();
        let _span = info_span!("mark_ol_checkpoint_l1_observed", epoch = tip.epoch).entered();
        let commitment = EpochCommitment::from_terminal(tip.epoch, *tip.l2_commitment());

        let block = self
            .ctx
            .get_l1_block(asm_block.blkid())
            .with_context(|| format!("fetching L1 block {asm_block} for checkpoint observation"))?;

        // Prepare for same checkpoint validation that ASM does.
        let parent_block = parent_commitment(asm_block, &block)?;
        let parent_asm_state = self.ctx.get_asm_state(&parent_block).with_context(|| {
            format!("fetching parent ASM state {parent_block} for checkpoint observation")
        })?;
        let mut checkpoint_state = decode_checkpoint_section(&parent_asm_state)?;
        let aux_data = self.ctx.get_aux_data(asm_block).with_context(|| {
            format!("fetching ASM aux data {asm_block} for checkpoint observation")
        })?;
        let verified_aux_data = VerifiedAuxData::try_new(
            &aux_data,
            &parent_asm_state.state().chain_view.history_accumulator,
        )
        .with_context(|| format!("verifying ASM aux data for {asm_block}"))?;

        let extracted = extract_matching_checkpoint(
            &block,
            self.ctx.magic_bytes(),
            tip,
            &mut checkpoint_state,
            asm_block.height(),
            &verified_aux_data,
        )
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no checkpoint envelope tx in L1 block {asm_block} validated for epoch {}",
                tip.epoch
            )
        })?;

        let observation = CheckpointL1Ref::new(*asm_block, extracted.txid, extracted.wtxid);
        self.ctx.put_checkpoint_l1_observation(
            commitment,
            extracted.payload,
            observation.clone(),
        )?;

        // Update cached confirmed epoch monotonically.
        if self
            .staged
            .confirmed_epoch
            .is_none_or(|current| commitment.epoch() > current.epoch())
        {
            self.staged.confirmed_epoch = Some(commitment);
        }

        // Queue only non-finalized candidates and avoid duplicates.
        if self
            .staged
            .finalized_epoch
            .is_none_or(|current| commitment.epoch() > current.epoch())
            && !self
                .staged
                .observed_checkpoints
                .iter()
                .any(|(epoch, _)| *epoch == commitment)
        {
            self.staged
                .observed_checkpoints
                .push_back((commitment, observation.clone()));
        }

        debug!(
            ?commitment,
            l1_height = asm_block.height(),
            txid = ?extracted.txid,
            wtxid = ?extracted.wtxid,
            "Recorded OL checkpoint L1 ref from tip update"
        );
        Ok(observation)
    }
}

/// Build a compatibility synthetic [`L1Checkpoint`] from a checkpoint tip update.
// TODO(STR-2438): Remove this adapter once CSM consumes checkpoint tip update
// structures directly without legacy `L1Checkpoint` synthesis.
fn checkpoint_from_tip_update(
    checkpoint_tip_update: &CheckpointTipUpdate,
    asm_block: &L1BlockCommitment,
    l1_reference: CheckpointL1Ref,
) -> L1Checkpoint {
    let tip = checkpoint_tip_update.tip();

    // TODO(STR-2438): This `BatchInfo` synthesis is semantically incorrect
    // for checkpoint tip updates (start/end L1 and L2 commitments are
    // duplicated placeholders). Replace with native checkpoint data flow
    // and remove legacy `BatchInfo` construction.
    let batch_info = BatchInfo::new(
        tip.epoch,
        (*asm_block, *asm_block),
        (*tip.l2_commitment(), *tip.l2_commitment()),
    );

    L1Checkpoint::new(batch_info, l1_reference)
}

/// Returns the parent L1 commitment derived from `block`'s header.
///
/// Fails for the genesis block where no parent exists; that case isn't
/// reachable in practice because epoch 0 produces no checkpoint tip update.
fn parent_commitment(
    asm_block: &L1BlockCommitment,
    block: &bitcoin::Block,
) -> anyhow::Result<L1BlockCommitment> {
    let height = asm_block.height();
    let parent_height = height
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("cannot derive parent for genesis L1 block {asm_block}"))?;
    let parent_blkid = L1BlockId::from(Buf32::from(block.header.prev_blockhash.to_byte_array()));
    Ok(L1BlockCommitment::new(parent_height, parent_blkid))
}

/// Extracts the checkpoint subprotocol's typed state from a parent `AsmState`.
fn decode_checkpoint_section(asm_state: &AsmState) -> anyhow::Result<CheckpointState> {
    asm_state
        .state()
        .find_section(CheckpointSubprotocol::ID)
        .ok_or_else(|| anyhow::anyhow!("checkpoint subprotocol section missing in ASM state"))?
        .try_to_state::<CheckpointSubprotocol>()
        .map_err(|e| anyhow::anyhow!("decode checkpoint subprotocol state: {e}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bitcoin::Network;
    use strata_asm_common::{
        AnchorState, AsmHistoryAccumulatorState, AsmLogEntry, ChainViewState,
        HeaderVerificationState,
    };
    use strata_asm_logs::constants::DEPOSIT_LOG_TYPE_ID;
    use strata_btc_verification::L1Anchor;
    use strata_csm_types::{ClientState, ClientUpdateOutput};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_l1_txfmt::MagicBytes;
    use strata_params::Params;
    use strata_primitives::prelude::*;
    use strata_state::asm_state::AsmState;
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
    use strata_test_utils::ArbitraryGenerator;

    use crate::{state::CsmWorkerState, test_utils::StubCtx};

    /// Builds a minimal `AsmState` carrying `logs`. The anchor state itself is
    /// inert — gap-fill only reads `AsmState::logs()`.
    fn make_asm_state(logs: Vec<AsmLogEntry>) -> AsmState {
        let anchor = L1Anchor {
            block: L1BlockCommitment::default(),
            next_target: 0,
            epoch_start_timestamp: 0,
            network: Network::Bitcoin,
        };
        let anchor_state = AnchorState {
            magic: AnchorState::magic_ssz(MagicBytes::from(*b"ALPN")),
            chain_view: ChainViewState {
                pow_state: HeaderVerificationState::init(anchor),
                history_accumulator: AsmHistoryAccumulatorState::new(0),
            },
            sections: Default::default(),
        };
        AsmState::new(anchor_state, logs)
    }

    /// A canonical L1 block id deterministically derived from a height, so
    /// tests can register and resolve gap blocks consistently.
    fn block_id_at(height: u32) -> L1BlockId {
        L1BlockId::from(Buf32::from([height as u8; 32]))
    }

    fn create_test_params_arc() -> Arc<Params> {
        Arc::new(strata_test_utils_l2::gen_params())
    }

    /// Sets up storage seeded with an empty client state and a fresh status channel.
    fn create_test_storage() -> (Arc<strata_storage::NodeStorage>, Arc<StatusChannel>) {
        let db = get_test_sled_backend();
        let pool = threadpool::ThreadPool::new(4);
        let storage = Arc::new(create_node_storage(db, pool).expect("Failed to create storage"));

        let initial_state = ClientState::new(None, None);
        let initial_block = L1BlockCommitment::new(0, L1BlockId::default());
        storage
            .client_state()
            .put_update_blocking(
                &initial_block,
                ClientUpdateOutput::new(initial_state, vec![]),
            )
            .expect("Failed to initialize client state");

        let mut arbgen = ArbitraryGenerator::new();
        let status_channel = Arc::new(StatusChannel::new(
            arbgen.generate(),
            arbgen.generate(),
            arbgen.generate(),
            None,
            None,
        ));
        (storage, status_channel)
    }

    /// Builds a default `StubCtx` with a panicking `get_l1_block`.
    fn default_stub_ctx(
        params: &Params,
        storage: Arc<strata_storage::NodeStorage>,
        status_channel: Arc<StatusChannel>,
    ) -> StubCtx {
        StubCtx::new(
            storage,
            status_channel,
            params.rollup.l1_reorg_safe_depth,
            params.rollup.magic_bytes,
        )
    }

    /// Helper to create a test CSM worker state with the default panicking stub ctx.
    fn create_test_state() -> (CsmWorkerState<StubCtx>, Arc<strata_storage::NodeStorage>) {
        let params = create_test_params_arc();
        let (storage, status_channel) = create_test_storage();
        let ctx = default_stub_ctx(&params, storage.clone(), status_channel);
        let state = CsmWorkerState::new(params, storage.clone(), ctx).unwrap();
        (state, storage)
    }

    /// Like [`create_test_state`] but lets the caller customize the `StubCtx`.
    fn create_test_state_with_ctx<F>(
        configure: F,
    ) -> (CsmWorkerState<StubCtx>, Arc<strata_storage::NodeStorage>)
    where
        F: FnOnce(StubCtx) -> StubCtx,
    {
        let params = create_test_params_arc();
        let (storage, status_channel) = create_test_storage();
        let ctx = configure(default_stub_ctx(&params, storage.clone(), status_channel));
        let state = CsmWorkerState::new(params, storage.clone(), ctx).unwrap();
        (state, storage)
    }

    /// Helper to create a known non-checkpoint log type entry.
    fn create_non_checkpoint_log_type() -> AsmLogEntry {
        let mut arbgen = ArbitraryGenerator::new();
        let payload = (0..8).map(|_| arbgen.generate()).collect::<Vec<u8>>();
        AsmLogEntry::from_msg(DEPOSIT_LOG_TYPE_ID, payload)
            .expect("Failed to create non-checkpoint log")
    }

    /// Helper to create a log entry without a type
    fn create_typeless_log() -> AsmLogEntry {
        let mut arbgen = ArbitraryGenerator::new();
        // Keep raw length below TypeId width so this remains typeless by construction.
        AsmLogEntry::from_raw(vec![arbgen.generate::<u8>()])
            .expect("single-byte raw payload should produce typeless log")
    }

    #[test]
    fn test_process_log_with_non_checkpoint_log_type() {
        let (mut state, _) = create_test_state();
        let asm_block = L1BlockCommitment::new(100, L1BlockId::default());

        let log = create_non_checkpoint_log_type();

        // Should succeed but do nothing
        let result = state.process_log(&log, &asm_block);
        assert!(
            result.is_ok(),
            "process_log should ignore known non-checkpoint log types"
        );

        // State should not be updated
        assert_eq!(state.staged.last_processed_epoch, None);
    }

    #[test]
    fn test_process_log_with_no_log_type() {
        let (mut state, _) = create_test_state();
        let asm_block = L1BlockCommitment::new(100, L1BlockId::default());

        let log = create_typeless_log();

        // Should succeed but do nothing
        let result = state.process_log(&log, &asm_block);
        assert!(result.is_ok(), "process_log should handle typeless logs");

        // State should not be updated
        assert_eq!(state.staged.last_processed_epoch, None);
    }

    /// Synthesizes a CheckpointTipUpdate log for `epoch`. We use a placeholder
    /// `CheckpointTip` because tests using this helper short-circuit before
    /// reaching extraction, so the tip content is irrelevant.
    fn placeholder_tip_log(epoch: u32) -> (AsmLogEntry, OLBlockCommitment) {
        use strata_asm_logs::CheckpointTipUpdate;
        use strata_asm_proto_checkpoint_types::CheckpointTip;
        let ol_tip = OLBlockCommitment::new(
            epoch as u64 * 10,
            OLBlockId::from(Buf32::from([epoch as u8; 32])),
        );
        let log = AsmLogEntry::from_log(&CheckpointTipUpdate::new(CheckpointTip::new(
            epoch, 200, ol_tip,
        )))
        .expect("tip log");
        (log, ol_tip)
    }

    #[test]
    fn errors_when_l1_block_fetch_fails() {
        let epoch = 9u32;
        let (log, ol_tip) = placeholder_tip_log(epoch);
        let asm_block = L1BlockCommitment::new(250, L1BlockId::default());

        let (mut state, storage) = create_test_state_with_ctx(|c| c.with_l1_fetch_failure());
        state.last_asm_block = Some(asm_block);
        let err = state
            .process_log(&log, &asm_block)
            .expect_err("fetch failure should propagate");
        assert!(
            err.to_string().contains("fetching L1 block"),
            "unexpected error: {err}"
        );

        let commitment = EpochCommitment::from_terminal(epoch, ol_tip);
        let observation = storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_blocking(commitment)
            .expect("query l1 ref");
        assert!(
            observation.is_none(),
            "no l1 ref should be written on fetch failure"
        );
    }

    #[test]
    fn commit_block_persists_state_and_advances_cursor() {
        let (mut state, storage) = create_test_state();
        let asm_block = L1BlockCommitment::new(300, L1BlockId::from(Buf32::from([7; 32])));

        state
            .commit_block(asm_block)
            .expect("commit should succeed");

        assert_eq!(state.last_asm_block, Some(asm_block));
        let (persisted_block, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row should exist");
        assert_eq!(
            persisted_block, asm_block,
            "client-state row must be keyed on the committed block"
        );
    }

    #[test]
    fn failed_log_does_not_advance_persisted_cursor() {
        let epoch = 9u32;
        let (log, _ol_tip) = placeholder_tip_log(epoch);
        let asm_block = L1BlockCommitment::new(250, L1BlockId::default());

        let (mut state, storage) = create_test_state_with_ctx(|c| c.with_l1_fetch_failure());
        state.last_asm_block = Some(asm_block);

        // The genesis client-state row seeded by `create_test_storage`.
        let (before_block, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("seeded client state");

        // Simulate the service loop: a failing log means `commit_block` is
        // never called. Pin the failure to the L1-fetch path so the test
        // can't pass on an unrelated error.
        let err = state
            .process_log(&log, &asm_block)
            .expect_err("L1 fetch failure should make the log fail");
        assert!(
            err.to_string().contains("fetching L1 block"),
            "unexpected error: {err}"
        );

        let (after_block, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state still present");
        assert_eq!(
            before_block, after_block,
            "persisted cursor must not move when a block's log failed"
        );
    }

    /// A `commit_block` failure must roll back all staged state, not just
    /// `cur_state`.
    #[test]
    fn commit_failure_rolls_back_staged_state() {
        let (mut state, storage) = create_test_state_with_ctx(|c| c.with_commit_failure());
        let last = L1BlockCommitment::new(100, block_id_at(100));
        state.last_asm_block = Some(last);

        // Seed staged cursors with a known baseline so we can detect any
        // partial advancement that survives the failed commit.
        let baseline_epoch = EpochCommitment::from_terminal(
            7,
            OLBlockCommitment::new(70, OLBlockId::from(Buf32::from([7; 32]))),
        );
        state.staged.last_processed_epoch = Some(7);
        state.staged.confirmed_epoch = Some(baseline_epoch);
        state.staged.finalized_epoch = Some(baseline_epoch);
        let baseline = state.staged.clone();

        let next = L1BlockCommitment::new(101, block_id_at(101));
        let err = state
            .process_asm_block(next, &[create_non_checkpoint_log_type()])
            .expect_err("commit failure should propagate");
        assert!(
            err.to_string().contains("committing CSM block"),
            "unexpected error: {err}"
        );

        // Commit cursor pinned at the last committed block.
        assert_eq!(state.last_asm_block, Some(last));
        // Every staged field restored to the pre-block baseline.
        assert_eq!(
            state.staged.last_processed_epoch,
            baseline.last_processed_epoch
        );
        assert_eq!(state.staged.confirmed_epoch, baseline.confirmed_epoch);
        assert_eq!(state.staged.finalized_epoch, baseline.finalized_epoch);
        assert_eq!(
            state.staged.observed_checkpoints,
            baseline.observed_checkpoints
        );
        assert!(
            storage
                .client_state()
                .get_update_blocking(&next)
                .expect("query client state")
                .is_none(),
            "uncommitted block must not be persisted"
        );
    }

    /// A contiguous block (height == last + 1) is processed and committed
    /// directly, with no gap-fill.
    #[test]
    fn contiguous_block_commits_directly() {
        let (mut state, storage) = create_test_state();
        let last = L1BlockCommitment::new(100, block_id_at(100));
        state.last_asm_block = Some(last);

        let next = L1BlockCommitment::new(101, block_id_at(101));
        state
            .process_asm_block(next, &[create_non_checkpoint_log_type()])
            .expect("contiguous block should process");

        assert_eq!(state.last_asm_block, Some(next));
        let (persisted, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row");
        assert_eq!(persisted, next);
    }

    /// When the status channel skips blocks, gap-fill replays the missing ones
    /// from storage and commits each, so the persisted cursor advances one
    /// block at a time rather than jumping the gap.
    #[test]
    fn gapped_block_replays_skipped_blocks() {
        let last = L1BlockCommitment::new(100, block_id_at(100));
        // Status jumps from 100 to 104; blocks 101..=103 were skipped.
        let target = L1BlockCommitment::new(104, block_id_at(104));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            let mut c = c;
            for height in 101..=103 {
                c = c.with_canonical_asm_state(
                    height,
                    block_id_at(height),
                    make_asm_state(vec![create_non_checkpoint_log_type()]),
                );
            }
            c
        });
        state.last_asm_block = Some(last);

        state
            .process_asm_block(target, &[create_non_checkpoint_log_type()])
            .expect("gap-fill should replay skipped blocks and commit target");

        // Cursor advanced all the way to the target.
        assert_eq!(state.last_asm_block, Some(target));
        // The last persisted client-state row is keyed on the target block,
        // and each gap block was committed in turn (105 distinct keys exist).
        let (persisted, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row");
        assert_eq!(persisted, target);
        for height in 101..=104 {
            let block = L1BlockCommitment::new(height, block_id_at(height));
            assert!(
                storage
                    .client_state()
                    .get_update_blocking(&block)
                    .expect("query client state")
                    .is_some(),
                "gap block {height} should have been committed"
            );
        }
    }

    /// If a gap block can't be resolved, gap-fill fails and the persisted
    /// cursor stays pinned at the last contiguous block — a restart then
    /// re-processes from there instead of skipping the gap.
    #[test]
    fn gap_fill_failure_pins_cursor() {
        let last = L1BlockCommitment::new(100, block_id_at(100));
        let target = L1BlockCommitment::new(104, block_id_at(104));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            // Block 101 resolves, but 102 fails to resolve.
            c.with_canonical_asm_state(
                101,
                block_id_at(101),
                make_asm_state(vec![create_non_checkpoint_log_type()]),
            )
            .with_canonical_failure_at(102)
        });
        state.last_asm_block = Some(last);

        let (before_block, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("seeded client state");

        let err = state
            .process_asm_block(target, &[create_non_checkpoint_log_type()])
            .expect_err("gap-fill should fail when a gap block can't be resolved");
        assert!(
            err.to_string()
                .contains("resolving gap L1 block at height 102"),
            "unexpected error: {err}"
        );

        // Block 101 was committed before the failure; the cursor advanced to
        // 101 but no further — it did not jump to the target.
        assert_eq!(
            state.last_asm_block,
            Some(L1BlockCommitment::new(101, block_id_at(101)))
        );
        assert!(
            storage
                .client_state()
                .get_update_blocking(&target)
                .expect("query client state")
                .is_none(),
            "target block must not be committed when gap-fill fails"
        );
        // Genesis row is still the most recent fully-walkable anchor's
        // predecessor; the key point is the target was never persisted.
        let _ = before_block;
    }

    /// A same-height block with a different blkid is treated as a reorged
    /// tip: its logs are processed and the cursor advances to the new blkid.
    #[test]
    fn same_height_reorg_processes_new_tip() {
        let (mut state, storage) = create_test_state();
        let old_blkid = block_id_at(100);
        let new_blkid = block_id_at(0xFE);
        let last = L1BlockCommitment::new(100, old_blkid);
        state.last_asm_block = Some(last);

        let reorged = L1BlockCommitment::new(100, new_blkid);
        state
            .process_asm_block(reorged, &[create_non_checkpoint_log_type()])
            .expect("same-height reorg should process the new tip");

        assert_eq!(
            state.last_asm_block,
            Some(reorged),
            "cursor must advance to the reorged-in blkid"
        );
        let row = storage
            .client_state()
            .get_update_blocking(&reorged)
            .expect("query client state");
        assert!(
            row.is_some(),
            "client-state row must be persisted under the new blkid"
        );
    }

    /// A status for a block at or behind the last committed block is a no-op
    #[test]
    fn stale_or_duplicate_block_is_ignored() {
        let (mut state, storage) = create_test_state();
        let last = L1BlockCommitment::new(100, block_id_at(100));
        state.last_asm_block = Some(last);

        let (before, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("seeded client state");

        // Same height as last.
        state
            .process_asm_block(last, &[create_non_checkpoint_log_type()])
            .expect("duplicate block should be a no-op");
        // Behind last.
        let behind = L1BlockCommitment::new(50, block_id_at(50));
        state
            .process_asm_block(behind, &[create_non_checkpoint_log_type()])
            .expect("stale block should be a no-op");

        assert_eq!(state.last_asm_block, Some(last));
        let (after, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state still present");
        assert_eq!(
            before, after,
            "no commit should happen for stale/dup blocks"
        );
    }
}
