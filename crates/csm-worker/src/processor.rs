//! Checkpoint log processing logic.

use std::sync::Arc;

use bitcoin::hashes::Hash;
use strata_asm_common::{AsmLogEntry, Subprotocol, VerifiedAuxData};
use strata_asm_logs::{CheckpointTipUpdate, constants::AsmLogTypeId};
use strata_asm_proto_checkpoint::{CheckpointState, CheckpointSubprotocol};
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
use strata_identifiers::Epoch;
use strata_primitives::{l1::is_l1_reorg_safe, prelude::*};
use strata_state::asm_state::AsmState;
use tracing::*;

use crate::{
    checkpoint_extract::{CheckpointVerificationContext, extract_matching_checkpoint},
    context::CsmWorkerContext,
    errors::{CsmWorkerError, CsmWorkerResult},
    state::{CsmWorkerState, derive_state},
};

/// The in-flight CSM update produced by processing one ASM block's logs.
pub(crate) struct PendingCsmUpdate {
    /// Client state being built up by an ASM block's logs.
    pub(crate) cur_state: Arc<ClientState>,

    /// Last epoch a checkpoint log was processed for.
    pub(crate) last_processed_epoch: Option<Epoch>,

    /// Checkpoint observations made during this block, applied to the worker's
    /// finality cursors only after a successful commit.
    pub(crate) observations: Vec<L1Checkpoint>,

    /// Per-block verification fixtures, built once on the first checkpoint tip log.
    pub(crate) ckpt_verification_ctx: Option<CheckpointVerificationContext>,
}

impl PendingCsmUpdate {
    pub(crate) fn new(cur_state: Arc<ClientState>, last_processed_epoch: Option<Epoch>) -> Self {
        Self {
            cur_state,
            last_processed_epoch,
            observations: Vec::new(),
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
        // `last_finalized_checkpoint` is left as the previous value here; the
        // service loop refreshes it once `advance_finalization` declares a new
        // finalized epoch based on L1 depth.
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
        let state = next_state.as_ref().clone();
        self.ctx
            .put_client_state_update(&asm_block, ClientUpdateOutput::new(state.clone(), vec![]))?;

        // The list stays contiguous: a commit either extends the tip by one or
        // replaces it at the same height (reorg). It never skips a height.
        let last = self
            .recent_asm_blocks
            .last()
            .expect("recent_asm_blocks is non-empty");
        let extends_tip = asm_block.height() == last.height() + 1;
        let same_height_reorg = asm_block.height() <= last.height();
        debug_assert!(
            extends_tip || same_height_reorg,
            "commit skipped a height: tip {last}, block {asm_block}"
        );

        self.recent_asm_blocks.push(asm_block);
        self.prune_below_reorg_floor();
        self.last_committed_state = next_state;
        // Publish the client state to status channel.
        self.ctx.publish_client_state(state, asm_block);
        Ok(())
    }

    /// Keeps only the `depth` newest blocks: index 0 becomes the reorg-safe
    /// floor, the deepest point a reorg could reach. Older entries can never be
    /// reorged out, so they are dropped.
    fn prune_below_reorg_floor(&mut self) {
        let depth = self.ctx.l1_reorg_safe_depth().max(1) as usize;
        let len = self.recent_asm_blocks.len();
        if len > depth {
            self.recent_asm_blocks.drain(..len - depth);
        }
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
        self.apply_observations(pending.observations);

        Ok(())
    }

    /// Folds checkpoint observations made during a successfully committed block
    /// into the worker's finality cursors.
    fn apply_observations(&mut self, observations: impl IntoIterator<Item = L1Checkpoint>) {
        for checkpoint in observations {
            self.apply_checkpoint_observation(checkpoint);
        }
    }

    /// Folds a single checkpoint observation into the worker's finality
    /// cursors: bumps `confirmed_epoch` monotonically and queues the
    /// observation for incremental depth-driven finalization if it's still
    /// ahead of `finalized_epoch` and not already present.
    fn apply_checkpoint_observation(&mut self, checkpoint: L1Checkpoint) {
        let commitment = EpochCommitment::from(&checkpoint);

        // Update cached confirmed epoch monotonically.
        if self
            .confirmed_epoch
            .is_none_or(|current| commitment.epoch() > current.epoch())
        {
            self.confirmed_epoch = Some(commitment);
        }

        // Queue only non-finalized candidates and avoid duplicates.
        if self
            .finalized_epoch
            .is_none_or(|current| commitment.epoch() > current.epoch())
            && !self
                .observed_checkpoints
                .iter()
                .any(|existing| EpochCommitment::from(existing) == commitment)
        {
            self.observed_checkpoints.push_back(checkpoint);
        }
    }

    /// Walks the observation queue and advances `finalized_epoch` for every
    /// candidate buried at least `finality_depth` deep under `current_l1_tip`.
    ///
    /// Returns the most recently finalized `L1Checkpoint` if `finalized_epoch`
    /// advanced, else `None`. The caller uses the returned record to refresh
    /// `ClientState.last_finalized_checkpoint` so it reflects depth-driven
    /// finality rather than the (since-removed) observation heuristic.
    pub(crate) fn advance_finalization(
        &mut self,
        current_l1_tip: L1Height,
    ) -> Option<L1Checkpoint> {
        let prev_finalized = self.finalized_epoch;
        let finality_depth = self.ctx.l1_reorg_safe_depth();
        let mut latest_finalized: Option<L1Checkpoint> = None;

        while let Some(candidate) = self.observed_checkpoints.front() {
            let commitment = EpochCommitment::from(candidate);
            if self
                .finalized_epoch
                .is_some_and(|current| commitment.epoch() <= current.epoch())
            {
                self.observed_checkpoints.pop_front();
                continue;
            }

            if !is_l1_reorg_safe(
                candidate.l1_reference.l1_commitment.height(),
                current_l1_tip,
                finality_depth,
            ) {
                break;
            }

            let finalized = self
                .observed_checkpoints
                .pop_front()
                .expect("front returned Some above");
            if self
                .finalized_epoch
                .is_none_or(|current| commitment.epoch() > current.epoch())
            {
                self.finalized_epoch = Some(commitment);
            }
            latest_finalized = Some(finalized);
        }

        if self.finalized_epoch != prev_finalized {
            latest_finalized
        } else {
            None
        }
    }

    /// Processes `asm_block` and its logs, first replaying any ASM blocks skipped
    /// between the last committed block and `asm_block`.
    ///
    ///  On error the cursor stays at the last contiguous block, so restarts resume safely.
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

        // A clean forward extension keeps `last` on the canonical chain.
        let last_still_canonical = self.ctx.get_canonical_l1_block(last.height())? == last;
        if !(asm_block.height() > last.height() && last_still_canonical) {
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
            let gap_block = self.ctx.get_canonical_l1_block(height)?;
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
    /// Errors if the fork lies at or below the finalized anchor (index 0): a
    /// reorg past finality is a protocol violation the worker must not absorb.
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
        self.recent_asm_blocks.truncate(fork_idx + 1);

        let fork_clstate = self
            .ctx
            .get_client_state_at(&fork_block)?
            .unwrap_or_default();
        let derived = derive_state(&self.ctx, &fork_block, &fork_clstate)?;
        let new_clstate = derived.new_clstate;
        self.confirmed_epoch = new_clstate.get_last_epoch();
        self.finalized_epoch = new_clstate.get_declared_final_epoch();
        self.observed_checkpoints = derived.observed_checkpoints;

        // Re-persist at the fork to overwrite the orphaned branch's row.
        self.ctx.put_client_state_update(
            &fork_block,
            ClientUpdateOutput::new_state(new_clstate.clone()),
        )?;
        self.ctx
            .publish_client_state(new_clstate.clone(), fork_block);
        self.last_committed_state = Arc::new(new_clstate);
        Ok(())
    }

    /// Highest list index whose block still matches the canonical L1 chain.
    ///
    /// `None` if even the finalized anchor (index 0) diverged.
    fn find_fork_index(&self) -> CsmWorkerResult<Option<usize>> {
        for idx in (0..self.recent_asm_blocks.len()).rev() {
            let block = self.recent_asm_blocks[idx];
            if self.ctx.get_canonical_l1_block(block.height())? == block {
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

    /// Validates a checkpoint tip update against the parent ASM state, writes
    /// the L1-ref observation to the DB, and updates the observations on the `pending` container.
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
        pending.observations.push(checkpoint.clone());

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
    use std::sync::Arc;

    use bitcoin::Network;
    use strata_asm_common::{
        AnchorState, AsmHistoryAccumulatorState, AsmLogEntry, ChainViewState,
        HeaderVerificationState,
    };
    use strata_asm_logs::constants::AsmLogTypeId;
    use strata_asm_params::AsmParams;
    use strata_btc_verification::L1Anchor;
    use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::RBuf32;
    use strata_l1_txfmt::MagicBytes;
    use strata_primitives::prelude::*;
    use strata_state::asm_state::AsmState;
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
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
        state.recent_asm_blocks = Some(asm_block);
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
        let asm_block = L1BlockCommitment::new(300, L1BlockId::from(Buf32::from([7; 32])));
        let next_state = state.last_committed_state.clone();

        state
            .commit_block(asm_block, next_state)
            .expect("commit should succeed");

        assert_eq!(state.recent_asm_blocks, Some(asm_block));
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
        state.recent_asm_blocks = Some(asm_block);

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
        let (mut state, storage) = create_test_state_with_ctx(|c| c.with_commit_failure());
        let last = L1BlockCommitment::new(100, block_id_at(100));
        state.recent_asm_blocks = Some(last);

        // Seed cursors with a known baseline so we can detect any partial
        // advancement that survives the failed commit.
        let baseline_epoch = EpochCommitment::from_terminal(
            7,
            OLBlockCommitment::new(70, OLBlockId::from(Buf32::from([7; 32]))),
        );
        state.last_processed_epoch = Some(7);
        state.confirmed_epoch = Some(baseline_epoch);
        state.finalized_epoch = Some(baseline_epoch);
        let baseline_last_processed_epoch = state.last_processed_epoch;
        let baseline_confirmed_epoch = state.confirmed_epoch;
        let baseline_finalized_epoch = state.finalized_epoch;
        let baseline_observed_checkpoints = state.observed_checkpoints.clone();

        let next = L1BlockCommitment::new(101, block_id_at(101));
        let err = state
            .process_asm_block(next, &[create_non_checkpoint_log_type()])
            .expect_err("commit failure should propagate");
        assert!(
            matches!(err, CsmWorkerError::Context(_)),
            "unexpected error: {err}"
        );

        // Commit cursor pinned at the last committed block.
        assert_eq!(state.recent_asm_blocks, Some(last));
        // Every cursor unchanged.
        assert_eq!(state.last_processed_epoch, baseline_last_processed_epoch);
        assert_eq!(state.confirmed_epoch, baseline_confirmed_epoch);
        assert_eq!(state.finalized_epoch, baseline_finalized_epoch);
        assert_eq!(state.observed_checkpoints, baseline_observed_checkpoints);
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
        state.recent_asm_blocks = Some(last);

        let next = L1BlockCommitment::new(101, block_id_at(101));
        state
            .process_asm_block(next, &[create_non_checkpoint_log_type()])
            .expect("contiguous block should process");

        assert_eq!(state.recent_asm_blocks, Some(next));
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
        state.recent_asm_blocks = Some(last);

        state
            .process_asm_block(target, &[create_non_checkpoint_log_type()])
            .expect("gap-fill should replay skipped blocks and commit target");

        // Cursor advanced all the way to the target.
        assert_eq!(state.recent_asm_blocks, Some(target));
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
        state.recent_asm_blocks = Some(last);

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
                CsmWorkerError::MissingData { what, ref detail }
                    if what == "canonical L1 block" && detail.contains("height 102")
            ),
            "unexpected error: {err}"
        );

        // Block 101 was committed before the failure; the cursor advanced to
        // 101 but no further — it did not jump to the target.
        assert_eq!(
            state.recent_asm_blocks,
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
        state.recent_asm_blocks = Some(last);

        let reorged = L1BlockCommitment::new(100, new_blkid);
        state
            .process_asm_block(reorged, &[create_non_checkpoint_log_type()])
            .expect("same-height reorg should process the new tip");

        assert_eq!(
            state.recent_asm_blocks,
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
        state.recent_asm_blocks = Some(last);

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

        assert_eq!(state.recent_asm_blocks, Some(last));
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

    /// Builds an observation queue entry at `epoch` anchored to L1 `height`.
    fn observation_at(epoch: u32, l1_height: L1Height) -> L1Checkpoint {
        let l2_commitment = OLBlockCommitment::new(
            epoch as u64 * 10,
            OLBlockId::from(Buf32::from([epoch as u8; 32])),
        );
        let tip = strata_asm_proto_checkpoint_types::CheckpointTip {
            epoch,
            l1_height,
            l2_commitment,
        };
        let l1_block = L1BlockCommitment::new(l1_height, block_id_at(l1_height));
        let l1_ref = CheckpointL1Ref::new(
            l1_block,
            RBuf32::from([epoch as u8; 32]),
            RBuf32::from([epoch as u8; 32]),
        );
        L1Checkpoint::new(tip, l1_ref)
    }

    /// Empty queue is a trivial no-op and never advances `finalized_epoch`.
    #[test]
    fn advance_finalization_empty_queue_is_noop() {
        let (mut state, _) = create_test_state();
        assert!(state.observed_checkpoints.is_empty());

        let finalized = state.advance_finalization(1_000);
        assert!(finalized.is_none());
        assert_eq!(state.finalized_epoch, None);
    }

    #[test]
    fn advance_finalization_below_depth_does_not_advance() {
        let (mut state, _) = create_test_state();
        state.observed_checkpoints.push_back(observation_at(1, 100));

        // tip at 101 has 2 confirmations, below threshold of 3.
        let finalized = state.advance_finalization(101);
        assert!(finalized.is_none());
        assert_eq!(state.finalized_epoch, None);
        assert_eq!(state.observed_checkpoints.len(), 1);
    }

    #[test]
    fn advance_finalization_single_advance() {
        let (mut state, _) = create_test_state();
        let candidate = observation_at(1, 100);
        let commitment = EpochCommitment::from(&candidate);
        state.observed_checkpoints.push_back(candidate.clone());

        // tip at 102 has 3 confirmations = depth threshold.
        let finalized = state.advance_finalization(102);
        assert_eq!(finalized.as_ref(), Some(&candidate));
        assert_eq!(state.finalized_epoch, Some(commitment));
        assert!(state.observed_checkpoints.is_empty());
    }

    #[test]
    fn advance_finalization_multi_advance_in_one_call() {
        let (mut state, _) = create_test_state();
        let candidate_1 = observation_at(1, 100);
        let candidate_2 = observation_at(2, 101);
        let commitment_2 = EpochCommitment::from(&candidate_2);
        state.observed_checkpoints.push_back(candidate_1);
        state.observed_checkpoints.push_back(candidate_2.clone());

        // tip at 103 so both candidates have ≥ 3 confirmations.
        let finalized = state.advance_finalization(103);
        assert_eq!(finalized.as_ref(), Some(&candidate_2));
        assert_eq!(state.finalized_epoch, Some(commitment_2));
        assert!(state.observed_checkpoints.is_empty());
    }

    #[test]
    fn advance_finalization_stops_at_shallow_candidate() {
        let (mut state, _) = create_test_state();
        let candidate_1 = observation_at(1, 100);
        let commitment_1 = EpochCommitment::from(&candidate_1);
        let candidate_2 = observation_at(2, 102);
        state.observed_checkpoints.push_back(candidate_1.clone());
        state.observed_checkpoints.push_back(candidate_2.clone());

        // tip at 102 so epoch 1 has 3 confirmations (buried), epoch 2 has 1.
        let finalized = state.advance_finalization(102);
        assert_eq!(finalized.as_ref(), Some(&candidate_1));
        assert_eq!(state.finalized_epoch, Some(commitment_1));
        assert_eq!(state.observed_checkpoints.len(), 1);
        assert_eq!(
            state
                .observed_checkpoints
                .front()
                .map(EpochCommitment::from),
            Some(EpochCommitment::from_terminal(
                2,
                OLBlockCommitment::new(20, OLBlockId::from(Buf32::from([2u8; 32])))
            ))
        );
    }

    #[test]
    fn advance_finalization_pops_already_finalized() {
        let (mut state, _) = create_test_state();
        let candidate = observation_at(1, 100);
        let commitment_1 = EpochCommitment::from(&candidate);
        state.finalized_epoch = Some(commitment_1);
        // Stale entry for an epoch ≤ finalized.
        state.observed_checkpoints.push_back(candidate);

        let finalized = state.advance_finalization(200);
        assert!(
            finalized.is_none(),
            "popping a stale entry must not register as an advance"
        );
        assert_eq!(state.finalized_epoch, Some(commitment_1));
        assert!(state.observed_checkpoints.is_empty());
    }
}
