//! Checkpoint log processing logic.

use std::sync::Arc;

use bitcoin::hashes::Hash;
use strata_asm_common::{AsmLogEntry, Subprotocol, VerifiedAuxData};
use strata_asm_logs::{CheckpointTipUpdate, constants::AsmLogTypeId};
use strata_asm_proto_checkpoint::{CheckpointState, CheckpointSubprotocol};
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
use strata_identifiers::Epoch;
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;
use tracing::*;

use crate::{
    checkpoint_extract::{CheckpointVerificationContext, extract_matching_checkpoint},
    context::CsmWorkerContext,
    errors::{CsmWorkerError, CsmWorkerResult},
    state::{CsmWorkerState, derive_state, reorg_floor_height},
};

/// The in-flight CSM update produced by processing one ASM block's logs.
pub(crate) struct PendingCsmUpdate {
    /// Client state being built up by an ASM block's logs.
    pub(crate) cur_state: Arc<ClientState>,

    /// Last epoch a checkpoint log was processed for.
    pub(crate) last_processed_epoch: Option<Epoch>,

    /// Per-block verification fixtures, built once on the first checkpoint tip log.
    pub(crate) ckpt_verification_ctx: Option<CheckpointVerificationContext>,
}

impl PendingCsmUpdate {
    pub(crate) fn new(cur_state: Arc<ClientState>, last_processed_epoch: Option<Epoch>) -> Self {
        Self {
            cur_state,
            last_processed_epoch,
            ckpt_verification_ctx: None,
        }
    }
}

impl<C: CsmWorkerContext> CsmWorkerState<C> {
    pub(crate) fn process_log(
        &self,
        pending: &mut PendingCsmUpdate,
        log: &AsmLogEntry,
        asm_block: &L1BlockCommitment,
    ) -> CsmWorkerResult<()> {
        match log.ty().and_then(|ty| AsmLogTypeId::try_from(ty).ok()) {
            Some(AsmLogTypeId::CheckpointTipUpdate) => {
                let tip_upd = log.try_into_log()?;

                return self.process_checkpoint_tip_log(pending, &tip_upd, asm_block);
            }
            None => {
                if let Some(log_type) = log.ty() {
                    debug!(log_type, "log type not processed by CSM");
                } else {
                    warn!("logs without a type ID?");
                }
            }
            Some(log_type) => {
                debug!(?log_type, "log type not processed by CSM");
            }
        }
        Ok(())
    }

    /// Process a checkpoint tip update log from the checkpoint subprotocol.
    fn process_checkpoint_tip_log(
        &self,
        pending: &mut PendingCsmUpdate,
        checkpoint_tip_update: &CheckpointTipUpdate,
        asm_block: &L1BlockCommitment,
    ) -> CsmWorkerResult<()> {
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
            warn!(
                tip_l1_height = l1_height,
                asm_block_height = asm_block.height(),
                "Checkpoint tip L1 height differs from current ASM block height; using ASM block commitment"
            );
        }

        let new_checkpoint =
            self.mark_ol_checkpoint_l1_observed(pending, checkpoint_tip_update, asm_block)?;
        pending.cur_state = Arc::new(ClientState::new(
            pending.cur_state.get_last_finalized_checkpoint(),
            Some(new_checkpoint),
        ));

        pending.last_processed_epoch = Some(epoch);
        Ok(())
    }

    /// Persists the client state for `asm_block` and advances the in-memory `last_asm_block`.
    ///
    /// Called once per block after every log was processed without error.
    fn commit_block(
        &mut self,
        asm_block: L1BlockCommitment,
        next_state: Arc<ClientState>,
    ) -> CsmWorkerResult<()> {
        // A commit either extends the tip by one or replaces it at the same
        // height (a reorg whose fork sits at the incoming height). It never
        // skips a height.
        let last = self
            .recent_asm_blocks
            .last()
            .expect("recent_asm_blocks is non-empty");
        let extends_tip = asm_block.height() == last.height() + 1;
        let same_height_reorg = asm_block.height() == last.height();
        debug_assert!(
            extends_tip || same_height_reorg,
            "received asm block skipping a height: tip {last}, block {asm_block}"
        );

        let state = derive_state(&self.ctx, &asm_block, &next_state)?;
        self.ctx
            .put_client_state_update(&asm_block, ClientUpdateOutput::new_state(state.clone()))?;
        self.recent_asm_blocks.push(asm_block);
        self.last_committed_state = Arc::new(state.clone());
        self.prune_below_reorg_floor();
        self.ctx.publish_client_state(state, asm_block);
        Ok(())
    }

    /// Drops blocks below the reorg-safe floor; index 0 stays the deepest point
    /// a reorg could reach.
    fn prune_below_reorg_floor(&mut self) {
        let Some(tip) = self.recent_asm_blocks.last() else {
            return;
        };
        debug_assert!(
            self.recent_asm_blocks
                .windows(2)
                .all(|w| w[0].height() <= w[1].height()),
            "recent_asm_blocks must be ascending for floor pruning"
        );
        let floor = reorg_floor_height(&self.ctx, &self.last_committed_state, tip.height());
        let keep_from = self
            .recent_asm_blocks
            .iter()
            .position(|b| b.height() >= floor)
            .unwrap_or(self.recent_asm_blocks.len());
        // Always retain at least the tip so the window is never empty.
        let keep_from = keep_from.min(self.recent_asm_blocks.len().saturating_sub(1));
        self.recent_asm_blocks.drain(..keep_from);
    }

    /// Processes every log of a single ASM block and commits it as one unit.
    ///
    /// All per-block work lives in [`PendingCsmUpdate`]; on any failure it is
    /// dropped and the worker's persistent fields are untouched.
    fn process_asm_logs(
        &mut self,
        asm_block: L1BlockCommitment,
        logs: &[AsmLogEntry],
    ) -> CsmWorkerResult<()> {
        let mut pending =
            PendingCsmUpdate::new(self.last_committed_state.clone(), self.last_processed_epoch);

        for log in logs {
            self.process_log(&mut pending, log, &asm_block)?;
        }

        self.commit_block(asm_block, pending.cur_state)?;

        // Commit succeeded; fold pending outputs onto the worker.
        self.last_processed_epoch = pending.last_processed_epoch;

        Ok(())
    }

    /// Processes `asm_block` and its logs, first replaying any ASM blocks skipped
    /// between the last committed block and `asm_block`.
    ///
    ///  On error the worker state stays the same, so restarts resume safely.
    pub(crate) fn process_asm_block(
        &mut self,
        asm_block: L1BlockCommitment,
        logs: &[AsmLogEntry],
    ) -> CsmWorkerResult<()> {
        let last = *self
            .recent_asm_blocks
            .last()
            .ok_or(CsmWorkerError::NoAnchorAsmBlock)?;

        // Exact duplicate — ASM redelivered the same status, nothing to do.
        if asm_block == last {
            debug!(%asm_block, "ASM status block matches last committed; skipping");
            return Ok(());
        }

        // A clean forward extension keeps `last` on the canonical chain. A
        // missing block (height above a reverted tip) means it diverged.
        let last_still_canonical = self.ctx.get_canonical_l1_block(last.height())? == Some(last);

        // Stale redelivery: a lower-height status while the tip is still
        // canonical is an out-of-order replay, not a reorg.
        if last_still_canonical && asm_block.height() < last.height() {
            debug!(%asm_block, %last, "ignoring stale lower-height ASM status");
            return Ok(());
        }

        let is_pure_extension = last_still_canonical && asm_block.height() > last.height();
        if !is_pure_extension {
            self.reorg_to_fork(&asm_block)?;
        }

        // Non-empty: `last` was extracted above and reorg keeps the fork entry.
        let resume_height = self
            .recent_asm_blocks
            .last()
            .expect("recent_asm_blocks is non-empty")
            .height();

        // Replay gap blocks, then the target.
        for height in (resume_height + 1)..asm_block.height() {
            let gap_block = self.ctx.get_canonical_l1_block(height)?.ok_or_else(|| {
                CsmWorkerError::MissingData {
                    what: "canonical L1 block",
                    detail: format!("height {height}"),
                }
            })?;
            let gap_state = self.ctx.get_asm_state(&gap_block)?;
            info!(%gap_block, "replaying ASM block skipped by status channel");
            self.process_asm_logs(gap_block, gap_state.logs())?;
        }
        // Process the target.
        self.process_asm_logs(asm_block, logs)?;
        Ok(())
    }

    /// Rewinds the worker to the fork point shared with the chain leading to
    /// `incoming`, re-deriving cursors and persisted state there.
    ///
    /// Errors if the fork lies at or below the finalized anchor (index 0).
    fn reorg_to_fork(&mut self, incoming: &L1BlockCommitment) -> CsmWorkerResult<()> {
        // No match means the divergence reaches past the finalized anchor.
        let Some(fork_idx) = self.find_fork_index()? else {
            return Err(CsmWorkerError::ReorgPastFinality {
                finalized: self.recent_asm_blocks[0],
                incoming: *incoming,
            });
        };
        let fork_block = self.recent_asm_blocks[fork_idx];

        warn!(%fork_block, %incoming, "reorg detected; rewinding to fork point");

        // It is expected to have client state persisted below the tip, which means below the fork
        // point as well.
        let persisted = self.ctx.get_client_state_at(&fork_block)?;
        let fork_clstate = persisted
            .clone()
            .ok_or_else(|| CsmWorkerError::MissingData {
                what: "client state at fork block",
                detail: fork_block.to_string(),
            })?;
        let new_clstate = derive_state(&self.ctx, &fork_block, &fork_clstate)?;

        // Re-persist at the fork to overwrite the orphaned branch's row.
        self.ctx.put_client_state_update(
            &fork_block,
            ClientUpdateOutput::new_state(new_clstate.clone()),
        )?;

        // Delete the orphaned branch's rows so a restart can't bootstrap from an
        // orphan that's above the canonical tip. A crash partway through is
        // recovered by bootstrap's `delete_orphan_rows_above` on restart.
        for orphan in &self.recent_asm_blocks[fork_idx + 1..] {
            self.ctx.del_client_state(orphan)?;
        }

        // Persistence done; commit the rewind to in-memory state.
        self.recent_asm_blocks.truncate(fork_idx + 1);

        // Publish if necessary.
        if persisted.as_ref() != Some(&new_clstate) {
            self.ctx
                .publish_client_state(new_clstate.clone(), fork_block);
        }
        self.last_committed_state = Arc::new(new_clstate);
        Ok(())
    }

    /// Highest list index whose block still matches the canonical L1 chain.
    fn find_fork_index(&self) -> CsmWorkerResult<Option<usize>> {
        for idx in (0..self.recent_asm_blocks.len()).rev() {
            let block = self.recent_asm_blocks[idx];
            // A missing block (above a reverted tip) or a mismatched blkid both
            // mean this entry diverged; keep walking down.
            if self.ctx.get_canonical_l1_block(block.height())? == Some(block) {
                return Ok(Some(idx));
            }
        }
        Ok(None)
    }

    fn get_checkpoint_verification_context(
        &self,
        asm_block: &L1BlockCommitment,
    ) -> CsmWorkerResult<CheckpointVerificationContext> {
        let block = self.ctx.get_l1_block(asm_block.blkid())?;

        // Prepare for same checkpoint validation that ASM does.
        let parent_block = parent_commitment(asm_block, &block)?;
        let parent_asm_state = self.ctx.get_asm_state(&parent_block)?;
        let checkpoint_state = decode_checkpoint_section(&parent_asm_state)?;
        let aux_data = self.ctx.get_aux_data(asm_block)?;
        let verified_aux_data = VerifiedAuxData::try_new(
            &aux_data,
            &parent_asm_state.state().chain_view.history_accumulator,
        )?;

        Ok(CheckpointVerificationContext {
            block,
            verified_aux_data,
            checkpoint_state,
            scan_cursor: 0,
        })
    }

    /// Validates a checkpoint tip update against the parent ASM state and writes
    /// the L1-ref observation to the DB.
    fn mark_ol_checkpoint_l1_observed(
        &self,
        pending: &mut PendingCsmUpdate,
        checkpoint_tip_update: &CheckpointTipUpdate,
        asm_block: &L1BlockCommitment,
    ) -> CsmWorkerResult<L1Checkpoint> {
        let tip = checkpoint_tip_update.tip();
        let _span = info_span!("mark_ol_checkpoint_l1_observed", epoch = tip.epoch).entered();
        let commitment = EpochCommitment::from_terminal(tip.epoch, *tip.l2_commitment());

        if pending.ckpt_verification_ctx.is_none() {
            pending.ckpt_verification_ctx =
                Some(self.get_checkpoint_verification_context(asm_block)?);
        }
        let ctx = pending
            .ckpt_verification_ctx
            .as_mut()
            .expect("just initialized above");

        let extracted =
            extract_matching_checkpoint(ctx, self.ctx.magic_bytes(), tip, asm_block.height())
                .ok_or(CsmWorkerError::NoMatchingCheckpoint {
                    asm_block: *asm_block,
                    epoch: tip.epoch,
                })?;

        let observation = CheckpointL1Ref::new(*asm_block, extracted.txid, extracted.wtxid);
        self.ctx.put_checkpoint_l1_observation(
            commitment,
            extracted.payload,
            observation.clone(),
        )?;

        let checkpoint = L1Checkpoint::new(*tip, observation);

        debug!(
            ?commitment,
            l1_height = asm_block.height(),
            txid = ?checkpoint.l1_reference.txid,
            wtxid = ?checkpoint.l1_reference.wtxid,
            "Recorded OL checkpoint L1 ref from tip update"
        );
        Ok(checkpoint)
    }
}

/// Returns the parent L1 commitment derived from `block`'s header.
///
/// Fails for the genesis block where no parent exists; that case isn't
/// reachable in practice because epoch 0 produces no checkpoint tip update.
fn parent_commitment(
    asm_block: &L1BlockCommitment,
    block: &bitcoin::Block,
) -> CsmWorkerResult<L1BlockCommitment> {
    let height = asm_block.height();
    let parent_height = height
        .checked_sub(1)
        .ok_or(CsmWorkerError::GenesisHasNoParent(*asm_block))?;
    let parent_blkid = L1BlockId::from(Buf32::from(block.header.prev_blockhash.to_byte_array()));
    Ok(L1BlockCommitment::new(parent_height, parent_blkid))
}

/// Extracts the checkpoint subprotocol's typed state from a parent `AsmState`.
fn decode_checkpoint_section(asm_state: &AsmState) -> CsmWorkerResult<CheckpointState> {
    asm_state
        .state()
        .find_section(CheckpointSubprotocol::ID)
        .ok_or(CsmWorkerError::MissingCheckpointSection)?
        .try_to_state::<CheckpointSubprotocol>()
        .map_err(CsmWorkerError::DecodeCheckpointSection)
}

#[cfg(test)]
mod tests {
    use std::{iter::once, sync::Arc};

    use bitcoin::Network;
    use strata_asm_common::{
        AnchorState, AsmHistoryAccumulatorState, AsmLogEntry, ChainViewState,
        HeaderVerificationState,
    };
    use strata_asm_logs::constants::AsmLogTypeId;
    use strata_asm_params::AsmParams;
    use strata_asm_proto_checkpoint_types::test_utils::create_test_checkpoint_payload;
    use strata_btc_verification::L1Anchor;
    use strata_checkpoint_types::EpochSummary;
    use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::RBuf32;
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::prelude::*;
    use strata_state::asm_state::AsmState;
    use strata_status::StatusChannel;
    use strata_storage::{NodeStorage, create_node_storage};
    use strata_test_utils::ArbitraryGenerator;

    use crate::{errors::CsmWorkerError, state::CsmWorkerState, test_utils::StubCtx};

    /// Reorg-safe depth used by the test stub context; the ASM params fixture
    /// carries no reorg depth (that lives in the node config at runtime).
    const TEST_L1_REORG_SAFE_DEPTH: u32 = 3;

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

    /// Persists an empty client-state row at `block`, mirroring what a prior
    /// CSM run leaves behind for every committed (at/below-tip) block. The reorg
    /// path re-derives the fork's state from this row.
    fn seed_client_state_row(storage: &strata_storage::NodeStorage, block: &L1BlockCommitment) {
        storage
            .client_state()
            .put_update_blocking(
                block,
                ClientUpdateOutput::new(ClientState::new(None, None), vec![]),
            )
            .expect("seed client state row");
    }

    fn create_test_params_arc() -> Arc<AsmParams> {
        Arc::new(strata_test_utils_l2::gen_asm_params())
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
        params: &AsmParams,
        storage: Arc<strata_storage::NodeStorage>,
        status_channel: Arc<StatusChannel>,
    ) -> StubCtx {
        StubCtx::new(
            storage,
            status_channel,
            TEST_L1_REORG_SAFE_DEPTH,
            params.magic,
            params.anchor.block,
        )
    }

    /// Helper to create a test CSM worker state with the default panicking stub ctx.
    fn create_test_state() -> (CsmWorkerState<StubCtx>, Arc<strata_storage::NodeStorage>) {
        let params = create_test_params_arc();
        let (storage, status_channel) = create_test_storage();
        let ctx = default_stub_ctx(&params, storage.clone(), status_channel);
        let state = CsmWorkerState::bootstrap(ctx).unwrap();
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
        let state = CsmWorkerState::bootstrap(ctx).unwrap();
        (state, storage)
    }

    /// Helper to create a known non-checkpoint log type entry.
    fn create_non_checkpoint_log_type() -> AsmLogEntry {
        let mut arbgen = ArbitraryGenerator::new();
        let payload = (0..8).map(|_| arbgen.generate()).collect::<Vec<u8>>();
        AsmLogEntry::from_msg(AsmLogTypeId::Deposit.into(), payload)
            .expect("Failed to create non-checkpoint log")
    }

    /// Helper to create a log entry without a type
    fn create_typeless_log() -> AsmLogEntry {
        let mut arbgen = ArbitraryGenerator::new();
        // Keep raw length below TypeId width so this remains typeless by construction.
        AsmLogEntry::from_raw(vec![arbgen.generate::<u8>()])
            .expect("single-byte raw payload should produce typeless log")
    }

    /// Builds a fresh `PendingCsmUpdate` from `state`'s last-committed values; used
    /// by tests that call into the per-block log methods directly.
    fn fresh_pending(state: &CsmWorkerState<StubCtx>) -> super::PendingCsmUpdate {
        super::PendingCsmUpdate::new(
            state.last_committed_state.clone(),
            state.last_processed_epoch,
        )
    }

    #[test]
    fn test_process_log_with_non_checkpoint_log_type() {
        let (state, _) = create_test_state();
        let asm_block = L1BlockCommitment::new(100, L1BlockId::default());

        let log = create_non_checkpoint_log_type();
        let mut pending = fresh_pending(&state);

        // Should succeed but do nothing
        let result = state.process_log(&mut pending, &log, &asm_block);
        assert!(
            result.is_ok(),
            "process_log should ignore known non-checkpoint log types"
        );

        // Pending update should not be touched.
        assert_eq!(pending.last_processed_epoch, None);
    }

    #[test]
    fn test_process_log_with_no_log_type() {
        let (state, _) = create_test_state();
        let asm_block = L1BlockCommitment::new(100, L1BlockId::default());

        let log = create_typeless_log();
        let mut pending = fresh_pending(&state);

        // Should succeed but do nothing
        let result = state.process_log(&mut pending, &log, &asm_block);
        assert!(result.is_ok(), "process_log should handle typeless logs");

        // Pending update should not be touched.
        assert_eq!(pending.last_processed_epoch, None);
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
        state.recent_asm_blocks = vec![asm_block];
        let mut pending = fresh_pending(&state);
        let err = state
            .process_log(&mut pending, &log, &asm_block)
            .expect_err("fetch failure should propagate");
        assert!(
            matches!(err, CsmWorkerError::L1Fetch { .. }),
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
        // Seed the list tip just below the block so the commit extends it
        // contiguously (commit_block asserts no height is skipped).
        state.recent_asm_blocks = vec![L1BlockCommitment::new(299, block_id_at(299))];
        let asm_block = L1BlockCommitment::new(300, L1BlockId::from(Buf32::from([7; 32])));
        let next_state = state.last_committed_state.clone();

        state
            .commit_block(asm_block, next_state)
            .expect("commit should succeed");

        assert_eq!(state.recent_asm_blocks.last(), Some(&asm_block));
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
        state.recent_asm_blocks = vec![asm_block];

        // The genesis client-state row seeded by `create_test_storage`.
        let (before_block, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("seeded client state");

        // Simulate the service loop: a failing log means `commit_block` is
        // never called. Pin the failure to the L1-fetch path so the test
        // can't pass on an unrelated error.
        let mut pending = fresh_pending(&state);
        let err = state
            .process_log(&mut pending, &log, &asm_block)
            .expect_err("L1 fetch failure should make the log fail");
        assert!(
            matches!(err, CsmWorkerError::L1Fetch { .. }),
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

    /// A `commit_block` failure must leave the worker's persistent cursors
    /// untouched. Because the per-block work lives in `PendingCsmUpdate` and is
    /// only folded back onto the worker after a successful commit, the
    /// pre-block values must be byte-identical after the failure.
    #[test]
    fn commit_failure_leaves_cursors_unchanged() {
        let last = L1BlockCommitment::new(100, block_id_at(100));
        let (mut state, storage) = create_test_state_with_ctx(|c| {
            c.with_commit_failure()
                .with_canonical_block(100, block_id_at(100))
        });
        state.recent_asm_blocks = vec![last];

        // Seed a cursor baseline so we can detect any partial advancement that
        // survives the failed commit.
        state.last_processed_epoch = Some(7);
        let baseline_last_processed_epoch = state.last_processed_epoch;
        let baseline_committed_state = state.last_committed_state.clone();

        let next = L1BlockCommitment::new(101, block_id_at(101));
        let err = state
            .process_asm_block(next, &[create_non_checkpoint_log_type()])
            .expect_err("commit failure should propagate");
        assert!(
            matches!(err, CsmWorkerError::Context(_)),
            "unexpected error: {err}"
        );

        // Commit cursor pinned at the last committed block.
        assert_eq!(state.recent_asm_blocks.last(), Some(&last));
        // Cursors unchanged.
        assert_eq!(state.last_processed_epoch, baseline_last_processed_epoch);
        assert_eq!(state.last_committed_state, baseline_committed_state);
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
        let last = L1BlockCommitment::new(100, block_id_at(100));
        let (mut state, storage) =
            create_test_state_with_ctx(|c| c.with_canonical_block(100, block_id_at(100)));
        state.recent_asm_blocks = vec![last];

        let next = L1BlockCommitment::new(101, block_id_at(101));
        state
            .process_asm_block(next, &[create_non_checkpoint_log_type()])
            .expect("contiguous block should process");

        assert_eq!(state.recent_asm_blocks.last(), Some(&next));
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
            let mut c = c.with_canonical_block(100, block_id_at(100));
            for height in 101..=103 {
                c = c.with_canonical_asm_state(
                    height,
                    block_id_at(height),
                    make_asm_state(vec![create_non_checkpoint_log_type()]),
                );
            }
            c
        });
        state.recent_asm_blocks = vec![last];

        state
            .process_asm_block(target, &[create_non_checkpoint_log_type()])
            .expect("gap-fill should replay skipped blocks and commit target");

        // Cursor advanced all the way to the target.
        assert_eq!(state.recent_asm_blocks.last(), Some(&target));
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
            c.with_canonical_block(100, block_id_at(100))
                .with_canonical_asm_state(
                    101,
                    block_id_at(101),
                    make_asm_state(vec![create_non_checkpoint_log_type()]),
                )
                .with_canonical_failure_at(102)
        });
        state.recent_asm_blocks = vec![last];

        let (before_block, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("seeded client state");

        let err = state
            .process_asm_block(target, &[create_non_checkpoint_log_type()])
            .expect_err("gap-fill should fail when a gap block can't be resolved");
        assert!(
            matches!(
                err,
                CsmWorkerError::MissingData { what, .. } if what == "canonical L1 block"
            ),
            "unexpected error: {err}"
        );

        // Block 101 was committed before the failure; the cursor advanced to
        // 101 but no further — it did not jump to the target.
        assert_eq!(
            state.recent_asm_blocks.last(),
            Some(&L1BlockCommitment::new(101, block_id_at(101)))
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

    // --- reorg handling ---

    #[test]
    fn duplicate_asm_block_is_noop() {
        let last = L1BlockCommitment::new(100, block_id_at(100));
        // No canonical block registered: any lookup would fail, proving the
        // duplicate path returns before consulting the canonical chain.
        let (mut state, storage) = create_test_state();
        state.recent_asm_blocks = vec![last];

        state
            .process_asm_block(last, &[create_non_checkpoint_log_type()])
            .expect("duplicate block should be a no-op and not error");

        assert_eq!(state.recent_asm_blocks, vec![last]);
        assert!(
            storage
                .client_state()
                .get_update_blocking(&last)
                .expect("query client state")
                .is_none(),
            "duplicate must not persist a client-state row"
        );
    }

    #[test]
    fn same_last_processed_height_reorg_replaces_tip() {
        let anchor = L1BlockCommitment::new(99, block_id_at(99));
        let orphan_100 = L1BlockCommitment::new(100, block_id_at(100));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            c.with_canonical_block(99, block_id_at(99))
                .with_canonical_block(100, block_id_at(101))
        });
        state.recent_asm_blocks = vec![anchor, orphan_100];

        // CSM persisted a client state at the fork block (anchor) in a prior
        // run; the reorg re-derives from it.
        seed_client_state_row(&storage, &anchor);
        // The orphaned tip left a row behind from the prior run.
        seed_client_state_row(&storage, &orphan_100);

        // Canonical chain: anchor stays at 99, and height 100 is now a
        // different block than the orphaned tip the worker committed.
        let canonical_100 = L1BlockCommitment::new(100, block_id_at(101));

        state
            .process_asm_block(canonical_100, &[create_non_checkpoint_log_type()])
            .expect("same-height reorg should rewind and re-commit");

        // Orphan tip dropped; canonical block now sits at height 100.
        assert_eq!(state.recent_asm_blocks.last(), Some(&canonical_100));
        assert!(
            !state.recent_asm_blocks.contains(&orphan_100),
            "orphaned tip must be pruned from the list"
        );
        assert!(
            storage
                .client_state()
                .get_update_blocking(&orphan_100)
                .expect("query client state")
                .is_none(),
            "orphaned tip's row must be deleted"
        );
        let (persisted, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row");
        assert_eq!(persisted, canonical_100);
    }

    #[test]
    fn reorg_truncates_to_fork_then_replays() {
        let anchor = L1BlockCommitment::new(100, block_id_at(100));
        let orphan_101 = L1BlockCommitment::new(101, block_id_at(101));
        let orphan_102 = L1BlockCommitment::new(102, block_id_at(102));
        let target = L1BlockCommitment::new(102, block_id_at(202));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            // Anchor at 100 stays canonical; 101 and 102 diverge.
            c.with_canonical_block(100, block_id_at(100))
                .with_canonical_block(101, block_id_at(201))
                .with_canonical_block(102, block_id_at(202))
                .with_canonical_asm_state(
                    101,
                    block_id_at(201),
                    make_asm_state(vec![create_non_checkpoint_log_type()]),
                )
        });
        state.recent_asm_blocks = vec![anchor, orphan_101, orphan_102];

        // CSM persisted a client state at the fork block in a prior run.
        seed_client_state_row(&storage, &anchor);

        state
            .process_asm_block(target, &[create_non_checkpoint_log_type()])
            .expect("reorg should truncate to fork and replay to target");

        assert_eq!(state.recent_asm_blocks.last(), Some(&target));
        assert!(
            !state.recent_asm_blocks.contains(&orphan_101)
                && !state.recent_asm_blocks.contains(&orphan_102),
            "orphaned branch must be pruned"
        );
        // Canonical 101 was replayed before the target.
        let (persisted, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row");
        assert_eq!(persisted, target);
        assert!(
            storage
                .client_state()
                .get_update_blocking(&L1BlockCommitment::new(101, block_id_at(201)))
                .expect("query client state")
                .is_some(),
            "canonical 101 should have been committed during replay"
        );
    }

    #[test]
    fn reorg_replays_gap_and_extends_past_old_tip() {
        let anchor = L1BlockCommitment::new(100, block_id_at(100));
        let orphan_101 = L1BlockCommitment::new(101, block_id_at(101));
        let orphan_102 = L1BlockCommitment::new(102, block_id_at(102));
        // New chain forks at 100 and runs longer, ending above the old tip (102).
        let target = L1BlockCommitment::new(105, block_id_at(205));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            // 100 stays canonical; 101..=104 are the new branch's gap blocks.
            let mut c = c.with_canonical_block(100, block_id_at(100));
            for height in 101..=104 {
                c = c.with_canonical_asm_state(
                    height,
                    block_id_at(100 + height),
                    make_asm_state(vec![create_non_checkpoint_log_type()]),
                );
            }
            c.with_canonical_block(105, block_id_at(205))
        });
        state.recent_asm_blocks = vec![anchor, orphan_101, orphan_102];

        // CSM persisted a client state at the fork block in a prior run.
        seed_client_state_row(&storage, &anchor);

        state
            .process_asm_block(target, &[create_non_checkpoint_log_type()])
            .expect("reorg should rewind, replay the gap, and extend past the old tip");

        // Tip landed at the new target, well past the old frontier.
        assert_eq!(state.recent_asm_blocks.last(), Some(&target));
        assert!(
            !state.recent_asm_blocks.contains(&orphan_101)
                && !state.recent_asm_blocks.contains(&orphan_102),
            "orphaned branch must be pruned"
        );
        // Each replayed gap block on the new chain got a committed row.
        for height in 101..=104 {
            let block = L1BlockCommitment::new(height, block_id_at(100 + height));
            assert!(
                storage
                    .client_state()
                    .get_update_blocking(&block)
                    .expect("query client state")
                    .is_some(),
                "new-chain gap block {height} should have been replayed and committed"
            );
        }
        // Most-recent persisted state is keyed at the target.
        let (persisted, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row");
        assert_eq!(persisted, target);
    }

    #[test]
    fn reorg_past_finality_errors() {
        let anchor = L1BlockCommitment::new(100, block_id_at(100));
        let tip = L1BlockCommitment::new(101, block_id_at(101));
        let incoming = L1BlockCommitment::new(101, block_id_at(201));

        // Even the anchor's height resolves to a different block, so no list
        // entry matches the canonical chain.
        let (mut state, _) = create_test_state_with_ctx(|c| {
            c.with_canonical_block(100, block_id_at(150))
                .with_canonical_block(101, block_id_at(201))
        });
        state.recent_asm_blocks = vec![anchor, tip];

        let err = state
            .process_asm_block(incoming, &[create_non_checkpoint_log_type()])
            .expect_err("reorg past finality should error");
        assert!(
            matches!(
                err,
                CsmWorkerError::ReorgPastFinality { finalized, incoming: inc }
                    if finalized == anchor && inc == incoming
            ),
            "unexpected error: {err}"
        );

        // Worker stays pinned at the original list.
        assert_eq!(state.recent_asm_blocks, vec![anchor, tip]);
    }

    #[test]
    fn reorg_anchors_at_highest_canonical_entry() {
        // Heights sit at/above the genesis anchor (40320) so nothing finalized
        // means pruning never drops a still-canonical entry; only the reorg
        // rewind shapes the list.
        let anchor = L1BlockCommitment::new(40320, block_id_at(20));
        let good_mid = L1BlockCommitment::new(40321, block_id_at(21));
        let orphan_tip = L1BlockCommitment::new(40322, block_id_at(22));
        let target = L1BlockCommitment::new(40322, block_id_at(202));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            // 40320 and 40321 still canonical; only 40322 diverged.
            c.with_canonical_block(40320, block_id_at(20))
                .with_canonical_block(40321, block_id_at(21))
                .with_canonical_block(40322, block_id_at(202))
        });
        state.recent_asm_blocks = vec![anchor, good_mid, orphan_tip];

        // CSM persisted a client state at the fork block (good_mid) in a prior run.
        seed_client_state_row(&storage, &good_mid);

        state
            .process_asm_block(target, &[create_non_checkpoint_log_type()])
            .expect("reorg should anchor at the highest canonical entry");

        // good_mid survived (fork point), orphan_tip replaced by target.
        assert_eq!(
            state.recent_asm_blocks,
            vec![anchor, good_mid, target],
            "list should rewind only to the highest canonical entry"
        );
    }

    #[test]
    fn reorg_repersists_state_at_fork() {
        let anchor = L1BlockCommitment::new(100, block_id_at(100));
        let orphan_tip = L1BlockCommitment::new(101, block_id_at(101));
        let incoming = L1BlockCommitment::new(101, block_id_at(201));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            c.with_canonical_block(100, block_id_at(100))
                .with_canonical_block(101, block_id_at(201))
        });
        state.recent_asm_blocks = vec![anchor, orphan_tip];

        // Seed a stale row at the fork block so we can see it overwritten.
        storage
            .client_state()
            .put_update_blocking(
                &anchor,
                ClientUpdateOutput::new(ClientState::new(None, None), vec![]),
            )
            .expect("seed fork row");

        state
            .process_asm_block(incoming, &[create_non_checkpoint_log_type()])
            .expect("reorg should re-persist at the fork");

        assert!(
            storage
                .client_state()
                .get_update_blocking(&anchor)
                .expect("query client state")
                .is_some(),
            "fork block must carry a re-derived client-state row"
        );
        assert_eq!(state.recent_asm_blocks.last(), Some(&incoming));
    }

    #[test]
    fn pure_extension_does_not_reorg() {
        // Heights sit at/above the genesis anchor (40320) so nothing finalized
        // means pruning leaves every entry in place.
        let anchor = L1BlockCommitment::new(40320, block_id_at(20));
        let last = L1BlockCommitment::new(40321, block_id_at(21));
        let next = L1BlockCommitment::new(40322, block_id_at(22));

        let (mut state, _) =
            create_test_state_with_ctx(|c| c.with_canonical_block(40321, block_id_at(21)));
        state.recent_asm_blocks = vec![anchor, last];

        state
            .process_asm_block(next, &[create_non_checkpoint_log_type()])
            .expect("pure extension should commit directly");

        // Both prior entries survive and the new block is appended.
        assert_eq!(state.recent_asm_blocks, vec![anchor, last, next]);
    }

    #[test]
    fn stale_lower_height_status_is_noop_when_tip_canonical() {
        let anchor = L1BlockCommitment::new(98, block_id_at(98));
        let mid = L1BlockCommitment::new(99, block_id_at(99));
        let tip = L1BlockCommitment::new(100, block_id_at(100));
        // A legitimate older canonical block (height 99) redelivered out of order.
        let stale = L1BlockCommitment::new(99, block_id_at(99));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            // Tip at 100 is still canonical (no reorg happened); 99 too.
            c.with_canonical_block(99, block_id_at(99))
                .with_canonical_block(100, block_id_at(100))
        });
        state.recent_asm_blocks = vec![anchor, mid, tip];

        state
            .process_asm_block(stale, &[create_non_checkpoint_log_type()])
            .expect("stale lower-height status must be a no-op");

        // Tip unchanged: not rewound onto the older block.
        assert_eq!(state.recent_asm_blocks.last(), Some(&tip));
        assert_eq!(state.recent_asm_blocks, vec![anchor, mid, tip]);
        // No row written at the stale height, and the cursor still sits at the tip.
        assert!(
            storage
                .client_state()
                .get_update_blocking(&stale)
                .expect("query client state")
                .is_none(),
            "stale block must not persist a client-state row"
        );
    }

    #[test]
    fn bootstrap_seeds_full_reorg_window() {
        let params = create_test_params_arc();
        let (storage, status_channel) = create_test_storage();

        // Nothing finalized, so the floor is the depth bound: with depth 3 and
        // tip 40325 the window spans [40323 ..= 40325].
        let floor = 40323;
        let tip = L1BlockCommitment::new(40325, block_id_at(25));
        // Store initial client state to db.
        storage
            .client_state()
            .put_update_blocking(
                &tip,
                ClientUpdateOutput::new(ClientState::new(None, None), vec![]),
            )
            .expect("seed client state at tip");

        // Build context with a canonical block at every intermediate height,
        // and at the tip so bootstrap takes the canonical (non-orphan) path.
        let mut ctx = default_stub_ctx(&params, storage.clone(), status_channel);
        for height in floor..tip.height() {
            ctx = ctx.with_canonical_block(height, block_id_at(height));
        }
        ctx = ctx.with_canonical_block(tip.height(), *tip.blkid());

        let state = CsmWorkerState::bootstrap(ctx).expect("bootstrap");

        let expected: Vec<_> = (floor..tip.height())
            .map(|height| L1BlockCommitment::new(height, block_id_at(height)))
            .chain(once(tip))
            .collect();
        assert_eq!(
            state.recent_asm_blocks, expected,
            "bootstrap must seed the full reorg-safe window"
        );
    }

    /// Seeds an OL-checkpoint observation (epoch summary + L1 ref + payload) at
    /// `l1_height`, mirroring the wiring `load_observed_checkpoints` reads.
    /// Returns the resulting epoch commitment.
    fn seed_ol_checkpoint_observation(
        storage: &NodeStorage,
        epoch: u32,
        l1_height: L1Height,
    ) -> EpochCommitment {
        let ol_checkpoint = storage.ol_checkpoint();
        let payload = create_test_checkpoint_payload(epoch);
        let ol_terminal = *payload.new_tip().l2_commitment();
        let summary = EpochSummary::new(
            epoch,
            ol_terminal,
            L2BlockCommitment::new(0, L2BlockId::default()),
            L1BlockCommitment::new(l1_height, block_id_at(l1_height)),
            Buf32::zero(),
        );
        let commitment = summary.get_epoch_commitment();
        ol_checkpoint
            .insert_epoch_summary_blocking(summary)
            .expect("insert epoch summary");
        ol_checkpoint
            .put_checkpoint_l1_observation_blocking(
                commitment,
                payload,
                CheckpointL1Ref::new(
                    L1BlockCommitment::new(l1_height, block_id_at(l1_height)),
                    RBuf32::from([epoch as u8; 32]),
                    RBuf32::from([epoch as u8; 32]),
                ),
            )
            .expect("insert epoch observation");
        commitment
    }

    /// Once a checkpoint finalizes via `commit_block`, the reorg window floor
    /// sits at the finalized height. A later reorg whose fork reaches below that
    /// floor finds no canonical match and is rejected as `ReorgPastFinality`,
    /// rather than rolling back finalized state.
    #[test]
    fn reorg_below_finalized_is_rejected() {
        // Heights sit above the genesis anchor (40320) so the genesis term of
        // the reorg floor never dominates the finalized/depth terms.
        let depth = TEST_L1_REORG_SAFE_DEPTH; // 3
        let floor_start: L1Height = 40399;
        let ckpt_height: L1Height = 40400;
        let finalize_tip: L1Height = ckpt_height + depth - 1; // 40402

        // Canonical chain diverges from the committed branch at every height in
        // the window (ids offset by 100), so the post-finalization reorg reaches
        // past the floor with no canonical match anywhere in the window.
        let (mut state, storage) = create_test_state_with_ctx(|c| {
            let mut c = c;
            for height in floor_start..=finalize_tip {
                c = c.with_canonical_block(height, block_id_at(height + 100));
            }
            c
        });

        // Seed a finalized window directly: the committed branch spans
        // floor_start..=finalize_tip, with epoch 1 (observed at ckpt_height)
        // already finalized.
        let commitment = seed_ol_checkpoint_observation(&storage, 1, ckpt_height);
        state.recent_asm_blocks = (floor_start..=finalize_tip)
            .map(|h| L1BlockCommitment::new(h, block_id_at(h)))
            .collect();
        let finalized = L1Checkpoint::new(
            *create_test_checkpoint_payload(1).new_tip(),
            storage
                .ol_checkpoint()
                .get_checkpoint_l1_ref_blocking(commitment)
                .expect("ref")
                .expect("seeded ref"),
        );
        state.last_committed_state = Arc::new(ClientState::new(Some(finalized), None));

        // Pruning against the finalized state pins the floor at the finalized
        // height, dropping the pre-finalization entry at 99.
        state.prune_below_reorg_floor();
        assert_eq!(
            state.recent_asm_blocks[0].height(),
            ckpt_height,
            "window floor must pin at the finalized height"
        );
        seed_client_state_row(&storage, &state.recent_asm_blocks[0]);

        // Incoming reorg at the tip height, on the canonical branch that
        // diverges from every committed entry down to and below the floor.
        let incoming = L1BlockCommitment::new(finalize_tip, block_id_at(finalize_tip + 100));
        let err = state
            .process_asm_block(incoming, &[create_non_checkpoint_log_type()])
            .expect_err("a reorg below finalized height must error");
        assert!(
            matches!(err, CsmWorkerError::ReorgPastFinality { .. }),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reorg_excludes_observations_above_fork() {
        let anchor = L1BlockCommitment::new(100, block_id_at(100));
        let orphan_tip = L1BlockCommitment::new(101, block_id_at(101));
        let incoming = L1BlockCommitment::new(101, block_id_at(201));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            // Fork lands at the anchor (height 100); 101 diverged. Epoch 2's
            // checkpoint rode the orphaned branch, so its canonical commitment
            // is gone after the reorg.
            c.with_canonical_block(100, block_id_at(100))
                .with_canonical_block(101, block_id_at(201))
                .with_orphaned_epoch(2)
        });
        state.recent_asm_blocks = vec![anchor, orphan_tip];

        // CSM persisted a client state at the fork block in a prior run.
        seed_client_state_row(&storage, &anchor);

        // Epoch 1 observed at L1 height 90; epoch 2 observed at the orphaned tip.
        let commitment_1 = seed_ol_checkpoint_observation(&storage, 1, 90);
        let commitment_2 = seed_ol_checkpoint_observation(&storage, 2, 101);

        state
            .process_asm_block(incoming, &[create_non_checkpoint_log_type()])
            .expect("reorg should re-derive state at the fork");

        // Epoch 1 is buried 100 - 90 = 10 >= depth, so it is both confirmed and
        // finalized. The orphaned epoch 2 must not leak into either.
        let clstate = &state.last_committed_state;
        assert_eq!(clstate.get_last_epoch(), Some(commitment_1));
        assert_eq!(clstate.get_declared_final_epoch(), Some(commitment_1));
        assert_ne!(clstate.get_last_epoch(), Some(commitment_2));
    }

    #[test]
    fn shorter_chain_reorg_rewinds_to_fork() {
        let anchor = L1BlockCommitment::new(40323, block_id_at(23));
        let good_24 = L1BlockCommitment::new(40324, block_id_at(24));
        let orphan_25 = L1BlockCommitment::new(40325, block_id_at(25));
        // New canonical tip is 40324; 40325 was reorged away. Incoming sits
        // below the old committed tip (40325) at a height that is canonical.
        let incoming = L1BlockCommitment::new(40324, block_id_at(24));

        let (mut state, storage) = create_test_state_with_ctx(|c| {
            // Canonical chain only reaches 40324; 40325 is not registered.
            c.with_canonical_block(40323, block_id_at(23))
                .with_canonical_block(40324, block_id_at(24))
        });
        state.recent_asm_blocks = vec![anchor, good_24, orphan_25];

        // CSM persisted a client state at the fork block (good_24) in a prior run.
        seed_client_state_row(&storage, &good_24);
        // The reorged-away tip left a higher-keyed row behind.
        seed_client_state_row(&storage, &orphan_25);

        state
            .process_asm_block(incoming, &[create_non_checkpoint_log_type()])
            .expect("shorter-chain reorg should rewind to the fork and commit");

        assert_eq!(state.recent_asm_blocks.last(), Some(&incoming));
        assert!(
            !state.recent_asm_blocks.contains(&orphan_25),
            "orphaned higher entry must be pruned"
        );
        assert!(
            storage
                .client_state()
                .get_update_blocking(&orphan_25)
                .expect("query client state")
                .is_none(),
            "orphaned higher row must be deleted"
        );
        let (persisted, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row");
        assert_eq!(persisted, incoming);
    }

    /// `commit_block` must never skip a height: a non-contiguous commit trips
    /// the contiguity assertion in debug builds.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "skipping a height")]
    fn commit_skipping_height_panics() {
        let (mut state, _) = create_test_state();
        // Tip at 100; committing 102 skips 101.
        state.recent_asm_blocks = vec![L1BlockCommitment::new(100, block_id_at(100))];
        let skipping = L1BlockCommitment::new(102, block_id_at(102));
        let next_state = state.last_committed_state.clone();

        let _ = state.commit_block(skipping, next_state);
    }
}
