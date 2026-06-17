//! CSM worker service state.

use std::{collections::VecDeque, sync::Arc};

use strata_csm_types::{ClientState, ClientUpdateOutput, L1Checkpoint};
use strata_identifiers::Epoch;
use strata_primitives::{l1::is_l1_reorg_safe, prelude::*};
use strata_service::ServiceState;
use tracing::warn;

use crate::{
    constants,
    context::CsmWorkerContext,
    errors::{CsmWorkerError, CsmWorkerResult},
};

/// Number of client-state rows fetched per batch while deleting orphans.
const ORPHAN_SCAN_BATCH: usize = 64;

/// State for the CSM worker service.
///
/// This state is used by the CSM worker which acts as a listener to ASM worker
/// status updates, processing checkpoint logs from the checkpoint subprotocol.
#[expect(
    missing_debug_implementations,
    reason = "context generic doesn't require Debug"
)]
pub struct CsmWorkerState<C: CsmWorkerContext> {
    /// External services and configuration.
    pub(crate) ctx: C,

    /// Recently processed ASM blocks, oldest first. Index 0 is the reorg-safe
    /// finalized anchor. Advanced only by a successful commit.
    pub(crate) recent_asm_blocks: Vec<L1BlockCommitment>,

    /// Last durably committed client state.
    pub(crate) last_committed_state: Arc<ClientState>,

    /// Last epoch we processed a checkpoint for.
    pub(crate) last_processed_epoch: Option<Epoch>,
}

impl<C: CsmWorkerContext> CsmWorkerState<C> {
    /// Bootstraps a new CSM worker state from worker context.
    ///
    /// Also eagerly updates, persists and publishes the client state.
    pub fn bootstrap(ctx: C) -> CsmWorkerResult<Self> {
        let (loaded_block, loaded_clstate) = ctx
            .fetch_most_recent_client_state()?
            .unwrap_or((ctx.genesis_l1_block(), ClientState::default()));

        // Resolve the canonical just in case the loaded one is already orphaned.
        let (recent_l1blk, recent_clstate) =
            resolve_canonical_tip(&ctx, loaded_block, loaded_clstate)?;

        // A resolved-down tip means the loaded block was an orphan; delete the
        // orphaned rows above the canonical tip.
        if recent_l1blk != loaded_block {
            delete_orphan_rows_above(&ctx, recent_l1blk)?;
        }

        let new_clstate = Arc::new(derive_state(&ctx, &recent_l1blk, &recent_clstate)?);

        // Persist and publish only when the derived state actually changed.
        if *new_clstate != recent_clstate {
            let new_update = ClientUpdateOutput::new_state(new_clstate.as_ref().clone());
            ctx.put_client_state_update(&recent_l1blk, new_update)?;
            ctx.publish_client_state(new_clstate.as_ref().clone(), recent_l1blk);
        }

        let recent_asm_blocks = init_recent_asm_blocks(&ctx, &new_clstate, recent_l1blk)?;

        Ok(Self {
            ctx,
            recent_asm_blocks,
            last_committed_state: new_clstate,
            last_processed_epoch: None,
        })
    }

    /// Get the last ASM block that was processed.
    pub fn get_last_asm_block(&self) -> Option<L1BlockCommitment> {
        self.recent_asm_blocks.last().copied()
    }
}

/// Resolves the latest committed block still on the canonical chain.
fn resolve_canonical_tip<C: CsmWorkerContext>(
    ctx: &C,
    candidate_blk: L1BlockCommitment,
    candidate_clstate: ClientState,
) -> CsmWorkerResult<(L1BlockCommitment, ClientState)> {
    let floor = reorg_floor_height(ctx, &candidate_clstate, candidate_blk.height());
    if candidate_blk.height() <= floor
        || ctx.get_canonical_l1_block(candidate_blk.height())? == Some(candidate_blk)
    {
        return Ok((candidate_blk, candidate_clstate));
    }

    // Iterate from the tip, the first appearing client state is the canonical state.
    for height in (floor..candidate_blk.height()).rev() {
        let Some(canonical) = ctx.get_canonical_l1_block(height)? else {
            continue;
        };
        if let Some(state) = ctx.get_client_state_at(&canonical)? {
            return Ok((canonical, state));
        }
    }

    Err(CsmWorkerError::MissingData {
        what: "canonical client state at or above reorg floor",
        detail: candidate_blk.to_string(),
    })
}

/// Deletes every persisted client-state row strictly above `canonical_tip`.
fn delete_orphan_rows_above<C: CsmWorkerContext>(
    ctx: &C,
    canonical_tip: L1BlockCommitment,
) -> CsmWorkerResult<()> {
    let mut cursor = canonical_tip;
    loop {
        let batch = ctx.get_client_state_blocks_from(cursor, ORPHAN_SCAN_BATCH)?;
        let full = batch.len() >= ORPHAN_SCAN_BATCH;
        let mut advanced = false;
        for block in batch {
            if block > canonical_tip {
                ctx.del_client_state(&block)?;
            }
            if block > cursor {
                cursor = block;
                advanced = true;
            }
        }
        // The scan is inclusive of `cursor`, so a batch that adds no higher
        // block means nothing remains above it.
        if !advanced || !full {
            break;
        }
    }
    Ok(())
}

/// Builds the recent-ASM-blocks list from the reorg-safe floor (index 0) up to
/// `tip`, the last block CSM committed client state for.
fn init_recent_asm_blocks<C: CsmWorkerContext>(
    ctx: &C,
    clstate: &ClientState,
    tip: L1BlockCommitment,
) -> CsmWorkerResult<Vec<L1BlockCommitment>> {
    let floor = reorg_floor_height(ctx, clstate, tip.height());

    let mut blocks = Vec::new();
    for height in floor..tip.height() {
        let block =
            ctx.get_canonical_l1_block(height)?
                .ok_or_else(|| CsmWorkerError::MissingData {
                    what: "canonical L1 block",
                    detail: format!("height {height}"),
                })?;
        blocks.push(block);
    }
    blocks.push(tip);
    Ok(blocks)
}

/// Deepest L1 height a reorg could reach under csm-observed `tip`.
///
/// A block is reorg-safe once it is either buried `depth` deep under `tip` or at
/// or below the last finalized checkpoint.
pub(crate) fn reorg_floor_height<C: CsmWorkerContext>(
    ctx: &C,
    clstate: &ClientState,
    tip: L1Height,
) -> L1Height {
    let depth = ctx.l1_reorg_safe_depth().max(1);
    let genesis = ctx.genesis_l1_block().height();
    let depth_floor = tip.saturating_sub(depth - 1);
    let checkpoint_floor = finalized_l1_height(clstate).unwrap_or(genesis);
    depth_floor.max(checkpoint_floor).max(genesis)
}

/// L1 height of the last finalized checkpoint, if any.
pub(crate) fn finalized_l1_height(clstate: &ClientState) -> Option<L1Height> {
    clstate
        .get_last_finalized_checkpoint()
        .map(|ckpt| ckpt.height())
}

/// Derives the client state from storage as of `cur_block`.
pub(crate) fn derive_state<C: CsmWorkerContext>(
    ctx: &C,
    cur_block: &L1BlockCommitment,
    cur_clstate: &ClientState,
) -> CsmWorkerResult<ClientState> {
    let current_csm_tip = cur_block.height();
    let finality_depth = ctx.l1_reorg_safe_depth().max(1);
    let last_finalized_epoch = cur_clstate.get_declared_final_epoch();
    let observation_start_epoch = last_finalized_epoch
        .map(|epoch| epoch.epoch().saturating_add(1))
        .unwrap_or(0);

    let observed_checkpoints =
        load_observed_checkpoints(ctx, observation_start_epoch, current_csm_tip)?;

    // Highest-epoch observation buried deep enough to be reorg-safe.
    let new_finalized_ckpt = observed_checkpoints
        .iter()
        .rev()
        .find(|ckpt| is_l1_reorg_safe(ckpt.height(), current_csm_tip, finality_depth))
        .filter(|obs_ckpt| {
            last_finalized_epoch.is_none_or(|last_fin| last_fin.epoch() < obs_ckpt.tip.epoch)
        })
        .cloned()
        .or_else(|| cur_clstate.get_last_finalized_checkpoint());

    // Confirmed: the latest observation seen on L1.
    let confirmed_ckpt = observed_checkpoints
        .back()
        .cloned()
        .or_else(|| new_finalized_ckpt.clone());

    Ok(ClientState::new(new_finalized_ckpt, confirmed_ckpt))
}

/// Loads observed checkpoint candidates from the OL checkpoint DB via `ctx`,
/// starting from `start_epoch`.
fn load_observed_checkpoints<C: CsmWorkerContext>(
    ctx: &C,
    start_epoch: Epoch,
    cur_csm_tip: L1Height,
) -> CsmWorkerResult<VecDeque<L1Checkpoint>> {
    let Some(last_ep_with_l1ref) = ctx.get_last_checkpoint_l1_ref_epoch()? else {
        return Ok(VecDeque::new());
    };
    let last_checkpoint_epoch = last_ep_with_l1ref.epoch();

    let mut observed = VecDeque::new();
    for epoch in start_epoch..=last_checkpoint_epoch {
        // No canonical commitment at this epoch is expected (e.g. orphaned), so
        // skip silently; a commitment with a missing observation is not.
        let Some(commitment) = ctx.get_canonical_epoch_commitment_at(epoch)? else {
            continue;
        };
        let Some(observation) = ctx.get_checkpoint_l1_ref(commitment)? else {
            warn!(?commitment, "canonical epoch missing its L1 ref; skipping");
            continue;
        };
        let Some(payload) = ctx.get_checkpoint_payload(commitment)? else {
            warn!(
                ?commitment,
                "canonical epoch missing its checkpoint payload; skipping"
            );
            continue;
        };

        // Heights are strictly increasing in epoch order, so once one exceeds
        // the tip every later one does too.
        if observation.l1_commitment.height() > cur_csm_tip {
            break;
        }

        observed.push_back(L1Checkpoint::new(*payload.new_tip(), observation));
    }

    Ok(observed)
}

impl<C: CsmWorkerContext + 'static> ServiceState for CsmWorkerState<C> {
    fn name(&self) -> &str {
        constants::SERVICE_NAME
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_asm_params::AsmParams;
    use strata_asm_proto_checkpoint_types::test_utils::create_test_checkpoint_payload;
    use strata_checkpoint_types::EpochSummary;
    use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{Buf32, L1BlockId, RBuf32};
    use strata_primitives::prelude::*;
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
    use strata_test_utils::ArbitraryGenerator;

    use super::{CsmWorkerState, derive_state, reorg_floor_height};
    use crate::test_utils::StubCtx;

    fn create_test_params() -> Arc<AsmParams> {
        Arc::new(strata_test_utils_l2::gen_asm_params())
    }

    /// Seeds an observed checkpoint at `epoch` anchored to L1 `l1_height` and
    /// returns its epoch commitment.
    fn seed_observation(
        storage: &strata_storage::NodeStorage,
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
            L1BlockCommitment::new(
                l1_height,
                L1BlockId::from(Buf32::from([l1_height as u8; 32])),
            ),
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
                    L1BlockCommitment::new(
                        l1_height,
                        L1BlockId::from(Buf32::from([l1_height as u8; 32])),
                    ),
                    RBuf32::from([epoch as u8; 32]),
                    RBuf32::from([epoch as u8; 32]),
                ),
            )
            .expect("insert epoch observation");
        commitment
    }

    /// Builds a `StubCtx` over freshly seeded storage with reorg depth 3.
    fn derive_ctx() -> (StubCtx, Arc<strata_storage::NodeStorage>) {
        let params = create_test_params();
        let (storage, status_channel) = create_test_storage_and_status(params.clone());
        let ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            3,
            params.magic,
            params.anchor.block,
        );
        (ctx, storage)
    }

    /// An observation only `depth - 1` deep is not yet finalized.
    #[test]
    fn derive_state_below_depth_does_not_finalize() {
        let (ctx, storage) = derive_ctx();
        seed_observation(&storage, 1, 100);

        // tip 101 -> 2 confirmations, below the depth-3 threshold.
        let tip = L1BlockCommitment::new(101, L1BlockId::default());
        let clstate = derive_state(&ctx, &tip, &ClientState::new(None, None)).expect("derive");

        assert_eq!(clstate.get_declared_final_epoch(), None);
    }

    /// With several safe observations, finality lands at the highest epoch and
    /// confirmed tracks the latest observation seen on L1.
    #[test]
    fn derive_state_finalizes_highest_safe() {
        let (ctx, storage) = derive_ctx();
        seed_observation(&storage, 1, 100);
        let commitment_2 = seed_observation(&storage, 2, 101);

        // tip 103 -> both observations are >= depth-3 deep.
        let tip = L1BlockCommitment::new(103, L1BlockId::default());
        let clstate = derive_state(&ctx, &tip, &ClientState::new(None, None)).expect("derive");

        assert_eq!(clstate.get_declared_final_epoch(), Some(commitment_2));
        assert_eq!(clstate.get_last_epoch(), Some(commitment_2));
    }

    /// Confirmed tracks an observation that is seen but not yet finalized, while
    /// finality stays at the deeper one.
    #[test]
    fn derive_state_confirmed_ahead_of_finalized() {
        let (ctx, storage) = derive_ctx();
        let commitment_1 = seed_observation(&storage, 1, 100);
        let commitment_2 = seed_observation(&storage, 2, 102);

        // tip 102 -> epoch 1 is 3 deep (finalized); epoch 2 is 1 deep (seen only).
        let tip = L1BlockCommitment::new(102, L1BlockId::default());
        let clstate = derive_state(&ctx, &tip, &ClientState::new(None, None)).expect("derive");

        assert_eq!(clstate.get_declared_final_epoch(), Some(commitment_1));
        assert_eq!(clstate.get_last_epoch(), Some(commitment_2));
    }

    /// A prior finalized checkpoint is retained when no newer observation is yet
    /// safe to finalize.
    #[test]
    fn derive_state_keeps_prior_finalized() {
        let (ctx, storage) = derive_ctx();
        let prior = seed_observation(&storage, 1, 100);

        // First derive at tip 102 finalizes epoch 1; reuse that as the baseline.
        let tip_1 = L1BlockCommitment::new(102, L1BlockId::default());
        let baseline = derive_state(&ctx, &tip_1, &ClientState::new(None, None)).expect("derive");
        assert_eq!(baseline.get_declared_final_epoch(), Some(prior));

        seed_observation(&storage, 2, 103);

        // tip 103: epoch 2 only 1 deep, so no new finality; prior must persist.
        let tip_2 = L1BlockCommitment::new(103, L1BlockId::default());
        let clstate = derive_state(&ctx, &tip_2, &baseline).expect("derive");

        assert_eq!(clstate.get_declared_final_epoch(), Some(prior));
    }

    fn create_test_storage_and_status(
        params: Arc<AsmParams>,
    ) -> (Arc<strata_storage::NodeStorage>, Arc<StatusChannel>) {
        let db = get_test_sled_backend();
        let pool = threadpool::ThreadPool::new(4);
        let storage = Arc::new(create_node_storage(db, pool).expect("Failed to create storage"));

        let tip_block = L1BlockCommitment::new(20, L1BlockId::default());
        storage
            .client_state()
            .put_update_blocking(
                &tip_block,
                ClientUpdateOutput::new(ClientState::new(None, None), vec![]),
            )
            .expect("Failed to initialize client state");

        let mut arbgen = ArbitraryGenerator::new();
        let status_channel = Arc::new(StatusChannel::new(
            arbgen.generate(),
            params.anchor.block,
            arbgen.generate(),
            None,
            None,
        ));

        (storage, status_channel)
    }

    #[test]
    fn test_state_new_bootstraps_confirmed_and_finalized_from_observations() {
        let params = create_test_params();
        let (storage, status_channel) = create_test_storage_and_status(params.clone());
        let ol_checkpoint = storage.ol_checkpoint();

        let payload_1 = create_test_checkpoint_payload(1);
        let ol_terminal_1 = *payload_1.new_tip().l2_commitment();
        let summary_1 = EpochSummary::new(
            1,
            ol_terminal_1,
            L2BlockCommitment::new(0, L2BlockId::default()),
            L1BlockCommitment::new(17, L1BlockId::default()),
            Buf32::zero(),
        );
        let commitment_1 = summary_1.get_epoch_commitment();
        ol_checkpoint
            .insert_epoch_summary_blocking(summary_1)
            .expect("insert epoch 1 summary");
        ol_checkpoint
            .put_checkpoint_l1_observation_blocking(
                commitment_1,
                payload_1,
                CheckpointL1Ref::new(
                    L1BlockCommitment::new(17, L1BlockId::default()),
                    RBuf32::from([1; 32]),
                    RBuf32::from([2; 32]),
                ),
            )
            .expect("insert epoch 1 observation");

        let payload_2 = create_test_checkpoint_payload(2);
        let ol_terminal_2 = *payload_2.new_tip().l2_commitment();
        let summary_2 = EpochSummary::new(
            2,
            ol_terminal_2,
            ol_terminal_1,
            L1BlockCommitment::new(19, L1BlockId::default()),
            Buf32::zero(),
        );
        let commitment_2 = summary_2.get_epoch_commitment();
        ol_checkpoint
            .insert_epoch_summary_blocking(summary_2)
            .expect("insert epoch 2 summary");
        ol_checkpoint
            .put_checkpoint_l1_observation_blocking(
                commitment_2,
                payload_2,
                CheckpointL1Ref::new(
                    L1BlockCommitment::new(19, L1BlockId::default()),
                    RBuf32::from([3; 32]),
                    RBuf32::from([4; 32]),
                ),
            )
            .expect("insert epoch 2 observation");

        // Finality lands at epoch 1 (L1 height 17), so the reorg window spans
        // [17 ..= 20]; register a canonical block at each intermediate height.
        let mut ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            4,
            params.magic,
            params.anchor.block,
        );
        for height in 17..20 {
            ctx =
                ctx.with_canonical_block(height, L1BlockId::from(Buf32::from([height as u8; 32])));
        }
        let state = CsmWorkerState::bootstrap(ctx).expect("state init");

        assert_eq!(
            state.last_committed_state.get_last_epoch(),
            Some(commitment_2)
        );

        // The in-memory `last_committed_state` must reflect the refreshed
        // finality so downstream readers (chain worker, RPC) immediately see
        // the worker's view rather than the stale on-disk value.
        assert_eq!(
            state.last_committed_state.get_declared_final_epoch(),
            Some(commitment_1),
            "bootstrap must refresh in-memory ClientState to the derived finality"
        );

        // The same refreshed state must be persisted at `cur_block` so the
        // next restart loads finality consistent with the worker's view —
        // without it, `fetch_most_recent_client_state` would return the stale
        // pre-refresh state and the candidate (already pruned from the queue)
        // could never be re-derived.
        let (persisted_block, persisted_state) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row");
        let cur_block = L1BlockCommitment::new(20, L1BlockId::default());
        assert_eq!(
            persisted_block, cur_block,
            "refreshed ClientState must be keyed on the same cur_block"
        );
        assert_eq!(
            persisted_state.get_declared_final_epoch(),
            Some(commitment_1)
        );
    }

    /// Bootstrap must not rewrite ClientState when the on-disk state already
    /// matches (or exceeds) the depth-derived finality — otherwise restarts
    /// would churn the storage with redundant rows.
    #[test]
    fn test_state_new_does_not_refresh_when_baseline_matches() {
        let params = create_test_params();
        let (storage, status_channel) = create_test_storage_and_status(params.clone());
        let ol_checkpoint = storage.ol_checkpoint();

        let payload_1 = create_test_checkpoint_payload(1);
        let ol_terminal_1 = *payload_1.new_tip().l2_commitment();
        let summary_1 = EpochSummary::new(
            1,
            ol_terminal_1,
            L2BlockCommitment::new(0, L2BlockId::default()),
            L1BlockCommitment::new(17, L1BlockId::default()),
            Buf32::zero(),
        );
        let commitment_1 = summary_1.get_epoch_commitment();
        ol_checkpoint
            .insert_epoch_summary_blocking(summary_1)
            .expect("insert epoch 1 summary");
        let l1_ref_1 = CheckpointL1Ref::new(
            L1BlockCommitment::new(17, L1BlockId::default()),
            RBuf32::from([1; 32]),
            RBuf32::from([2; 32]),
        );
        ol_checkpoint
            .put_checkpoint_l1_observation_blocking(
                commitment_1,
                payload_1.clone(),
                l1_ref_1.clone(),
            )
            .expect("insert epoch 1 observation");

        // Seed the on-disk ClientState so both checkpoints already reflect epoch 1.
        let epoch_1_ckpt = L1Checkpoint::new(*payload_1.new_tip(), l1_ref_1);
        let baseline = ClientState::new(Some(epoch_1_ckpt.clone()), Some(epoch_1_ckpt));
        let baseline_block = L1BlockCommitment::new(20, L1BlockId::default());
        storage
            .client_state()
            .put_update_blocking(
                &baseline_block,
                ClientUpdateOutput::new(baseline.clone(), vec![]),
            )
            .expect("seed baseline client state");

        // Baseline finality is epoch 1 (L1 height 17), so the reorg window
        // spans [17 ..= 20]; register a canonical block at each intermediate
        // height.
        let mut ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            4,
            params.magic,
            params.anchor.block,
        );
        for height in 17..20 {
            ctx =
                ctx.with_canonical_block(height, L1BlockId::from(Buf32::from([height as u8; 32])));
        }
        let state = CsmWorkerState::bootstrap(ctx).expect("state init");

        assert_eq!(
            state.last_committed_state.get_declared_final_epoch(),
            Some(commitment_1)
        );

        let (_, persisted_state) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row");
        assert_eq!(
            persisted_state, baseline,
            "bootstrap must not rewrite ClientState when baseline already matches"
        );
    }

    /// Builds a `StubCtx` rooted at genesis L1 height 0 with the given reorg depth.
    fn stub_ctx_at_genesis_zero(depth: u32) -> StubCtx {
        let params = create_test_params();
        let (storage, status_channel) = create_test_storage_and_status(params.clone());
        StubCtx::new(
            storage,
            status_channel,
            depth,
            params.magic,
            L1BlockCommitment::new(0, L1BlockId::default()),
        )
    }

    /// Builds a `ClientState` whose last finalized checkpoint sits at `l1_height`.
    fn finalized_state_at(l1_height: L1Height) -> ClientState {
        let payload = create_test_checkpoint_payload(1);
        let ckpt = L1Checkpoint::new(
            *payload.new_tip(),
            CheckpointL1Ref::new(
                L1BlockCommitment::new(l1_height, L1BlockId::default()),
                RBuf32::from([1; 32]),
                RBuf32::from([2; 32]),
            ),
        );
        ClientState::new(Some(ckpt.clone()), Some(ckpt))
    }

    /// When the depth bound is deeper (lower) than the finalized checkpoint, the
    /// depth term sets the floor.
    #[test]
    fn reorg_floor_height_depth_term_wins() {
        let depth = 5;
        let tip = 98;
        let depth_floor = tip - (depth - 1);
        let checkpoint = depth_floor - 4; // checkpoint sits below the depth floor

        let ctx = stub_ctx_at_genesis_zero(depth);
        let clstate = finalized_state_at(checkpoint);
        assert_eq!(reorg_floor_height(&ctx, &clstate, tip), depth_floor);
    }

    /// When the finalized checkpoint is deeper (higher) than the depth bound, the
    /// checkpoint term sets the floor.
    #[test]
    fn reorg_floor_height_checkpoint_term_wins() {
        let depth = 5;
        let tip = 99;
        let depth_floor = tip - (depth - 1);
        let checkpoint = depth_floor + 3; // checkpoint sits above the depth floor

        let ctx = stub_ctx_at_genesis_zero(depth);
        let clstate = finalized_state_at(checkpoint);
        assert_eq!(reorg_floor_height(&ctx, &clstate, tip), checkpoint);
    }

    /// A blkid deterministically derived from a byte tag.
    fn blkid(tag: u8) -> L1BlockId {
        L1BlockId::from(Buf32::from([tag; 32]))
    }

    /// Persists an empty client-state row at `block`.
    fn put_client_state_row(storage: &strata_storage::NodeStorage, block: &L1BlockCommitment) {
        storage
            .client_state()
            .put_update_blocking(
                block,
                ClientUpdateOutput::new(ClientState::new(None, None), vec![]),
            )
            .expect("put client state row");
    }

    /// A stored tip higher than the canonical tip (chain reverted below it) is an
    /// orphan; bootstrap must anchor to the highest canonical row, not the orphan.
    #[test]
    fn bootstrap_ignores_higher_orphan_after_shorter_reorg() {
        let params = create_test_params();
        let (storage, status_channel) = create_test_storage_and_status(params.clone());

        // Genesis 40320, depth 4: floor for tip 40330 is 40327, so 40330 is
        // above the floor and the orphan path is exercised.
        let canonical_tip = L1BlockCommitment::new(40329, blkid(201));
        let orphan = L1BlockCommitment::new(40330, blkid(202));

        put_client_state_row(&storage, &canonical_tip);
        put_client_state_row(&storage, &orphan);

        // Canonical chain only reaches 40329; nothing at the orphan's height.
        let mut ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            4,
            params.magic,
            params.anchor.block,
        );
        for height in 40326..=40329 {
            ctx = ctx.with_canonical_block(height, blkid(height as u8));
        }
        ctx = ctx.with_canonical_block(40329, *canonical_tip.blkid());

        let state = CsmWorkerState::bootstrap(ctx).expect("bootstrap");

        assert_eq!(state.recent_asm_blocks.last(), Some(&canonical_tip));
        assert_ne!(state.recent_asm_blocks.last(), Some(&orphan));
    }

    /// Bootstrap deletes orphan rows above the canonical tip, so a subsequent
    /// `fetch_most_recent_state` returns the canonical row rather than the orphan.
    #[test]
    fn bootstrap_deletes_orphan_rows() {
        let params = create_test_params();
        let (storage, status_channel) = create_test_storage_and_status(params.clone());

        let canonical_tip = L1BlockCommitment::new(40329, blkid(201));
        let orphan = L1BlockCommitment::new(40330, blkid(202));
        put_client_state_row(&storage, &canonical_tip);
        put_client_state_row(&storage, &orphan);

        let mut ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            4,
            params.magic,
            params.anchor.block,
        );
        for height in 40326..=40329 {
            ctx = ctx.with_canonical_block(height, blkid(height as u8));
        }
        ctx = ctx.with_canonical_block(40329, *canonical_tip.blkid());

        CsmWorkerState::bootstrap(ctx).expect("bootstrap");

        assert!(
            storage
                .client_state()
                .get_update_blocking(&orphan)
                .expect("query orphan")
                .is_none(),
            "orphan row must be deleted"
        );
        let (recent, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query")
            .expect("row");
        assert_eq!(recent, canonical_tip);
    }

    /// An orphan branch deeper than one scan batch is fully deleted, proving the
    /// batched scan terminates and does not truncate. Exercises the deletion
    /// helper directly, since `resolve_canonical_tip` caps orphan depth at the
    /// reorg window before bootstrap would reach this path.
    #[test]
    fn delete_orphan_rows_spans_multiple_batches() {
        let params = create_test_params();
        let (storage, status_channel) = create_test_storage_and_status(params.clone());

        let canonical_tip = L1BlockCommitment::new(40329, blkid(201));
        put_client_state_row(&storage, &canonical_tip);

        // Seed more orphan rows than a single scan batch holds.
        let orphan_count = (super::ORPHAN_SCAN_BATCH + 5) as u32;
        for i in 0..orphan_count {
            let orphan = L1BlockCommitment::new(
                40330 + i,
                L1BlockId::from(Buf32::from([(i % 251 + 3) as u8; 32])),
            );
            put_client_state_row(&storage, &orphan);
        }

        let ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            4,
            params.magic,
            params.anchor.block,
        );
        super::delete_orphan_rows_above(&ctx, canonical_tip).expect("delete orphans");

        let (recent, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query")
            .expect("row");
        assert_eq!(
            recent, canonical_tip,
            "every orphan row above the canonical tip must be deleted"
        );
    }

    /// Two rows at the same height: the orphan sorts higher so `get_latest`
    /// returns it, but bootstrap must anchor to the canonical-blkid block.
    #[test]
    fn bootstrap_ignores_same_height_orphan() {
        let params = create_test_params();
        let (storage, status_channel) = create_test_storage_and_status(params.clone());

        // Canonical fork point one below; the canonical row at the same height
        // sorts lower than the orphan, which `get_latest` returns.
        let height = 40330;
        let canonical = L1BlockCommitment::new(height, blkid(0x01));
        let orphan = L1BlockCommitment::new(height, blkid(0xff));
        let fork = L1BlockCommitment::new(height - 1, blkid(0x10));

        put_client_state_row(&storage, &fork);
        put_client_state_row(&storage, &canonical);
        put_client_state_row(&storage, &orphan);

        // Canonical chain: the fork at H-1 and a different blkid at H.
        let mut ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            4,
            params.magic,
            params.anchor.block,
        );
        for h in 40326..height {
            ctx = ctx.with_canonical_block(h, blkid(h as u8));
        }
        ctx = ctx.with_canonical_block(height - 1, *fork.blkid());
        ctx = ctx.with_canonical_block(height, *canonical.blkid());

        let state = CsmWorkerState::bootstrap(ctx).expect("bootstrap");

        assert_ne!(state.recent_asm_blocks.last(), Some(&orphan));
        assert_eq!(state.recent_asm_blocks.last(), Some(&fork));
    }
}
