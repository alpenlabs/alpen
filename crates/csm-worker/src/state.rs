//! CSM worker service state.

use std::{collections::VecDeque, sync::Arc};

use strata_csm_types::{ClientState, ClientUpdateOutput, L1Checkpoint};
use strata_identifiers::Epoch;
use strata_primitives::{l1::is_l1_reorg_safe, prelude::*};
use strata_service::ServiceState;

use crate::{constants, context::CsmWorkerContext, errors::CsmWorkerResult};

/// State for the CSM worker service.
///
/// This state is used by the CSM worker which acts as a listener to ASM worker
/// status updates, processing checkpoint logs from the checkpoint subprotocol.
///
/// Every field is either the last durably committed value or a running cursor
/// advanced only after a successful commit. Per-block scratch state lives in
/// `BlockScratch` and never touches this struct on failure.
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

    /// Latest checkpoint epoch observed on L1.
    pub(crate) confirmed_epoch: Option<EpochCommitment>,

    /// Latest checkpoint epoch finalized by L1 depth, derived from observation facts and tip.
    pub(crate) finalized_epoch: Option<EpochCommitment>,

    /// Ordered observed checkpoint candidates used for incremental depth derivation.
    ///
    /// Items are appended after a successful block commit and consumed as
    /// finalized depth progresses.
    pub(crate) observed_checkpoints: VecDeque<L1Checkpoint>,
}

impl<C: CsmWorkerContext> CsmWorkerState<C> {
    /// Bootstraps a new CSM worker state from worker context.
    ///
    /// Also eagerly updates, persists and publishes the client state.
    pub fn bootstrap(ctx: C) -> CsmWorkerResult<Self> {
        let (recent_l1blk, recent_clstate) = ctx
            .fetch_most_recent_client_state()?
            .unwrap_or((ctx.genesis_l1_block(), ClientState::default()));

        let derived = derive_state(&ctx, &recent_l1blk, &recent_clstate)?;

        let new_clstate = Arc::new(derived.new_clstate);

        // Persist and publish only when the derived state actually changed.
        if *new_clstate != recent_clstate {
            let new_update = ClientUpdateOutput::new_state(new_clstate.as_ref().clone());
            ctx.put_client_state_update(&recent_l1blk, new_update)?;
            ctx.publish_client_state(new_clstate.as_ref().clone(), recent_l1blk);
        }

        let recent_asm_blocks = init_recent_processed_blocks(&ctx, recent_l1blk)?;
        let confirmed_epoch = new_clstate.get_last_epoch();
        let finalized_epoch = new_clstate.get_declared_final_epoch();

        Ok(Self {
            ctx,
            recent_asm_blocks,
            last_committed_state: new_clstate,
            last_processed_epoch: None,
            confirmed_epoch,
            finalized_epoch,
            observed_checkpoints: derived.observed_checkpoints,
        })
    }

    /// Get the last ASM block that was processed.
    pub fn get_last_asm_block(&self) -> Option<L1BlockCommitment> {
        self.recent_asm_blocks.last().copied()
    }
}

/// Client state and surviving observation queue derived as of an L1 tip.
pub(crate) struct DerivedState {
    pub(crate) observed_checkpoints: VecDeque<L1Checkpoint>,
    /// Client state rebuilt from the derived checkpoints.
    pub(crate) new_clstate: ClientState,
}

/// Derives client state and newly observed checkpoints from storage as of `cur_block`.
pub(crate) fn derive_state<C: CsmWorkerContext>(
    ctx: &C,
    cur_block: &L1BlockCommitment,
    cur_clstate: &ClientState,
) -> CsmWorkerResult<DerivedState> {
    let current_l1_tip = cur_block.height();
    let finality_depth = ctx.l1_reorg_safe_depth().max(1);
    let last_finalized_epoch = cur_clstate.get_declared_final_epoch();
    let observation_start_epoch = last_finalized_epoch
        .map(|epoch| epoch.epoch().saturating_add(1))
        .unwrap_or(0);

    let mut observed_checkpoints =
        load_observed_checkpoints(ctx, observation_start_epoch, current_l1_tip)?;

    // Derive new finalized checkpoint from last finalized and finalized ones among the observed.
    let new_finalized_ckpt = observed_checkpoints
        .iter()
        .rev()
        .find(|ckpt| is_l1_reorg_safe(ckpt.height(), current_l1_tip, finality_depth))
        .cloned()
        .filter(|obs_ckpt| {
            last_finalized_epoch.is_none_or(|last_fin| last_fin.epoch() < obs_ckpt.tip.epoch)
        })
        .or_else(|| cur_clstate.get_last_finalized_checkpoint());

    // Confirmed: the latest observation seen on L1, else the finalized one.
    let confirmed_ckpt = observed_checkpoints
        .back()
        .cloned()
        .or_else(|| new_finalized_ckpt.clone());

    let finalized_epoch = new_finalized_ckpt.as_ref().map(EpochCommitment::from);
    let new_clstate = ClientState::new(new_finalized_ckpt, confirmed_ckpt);

    // Keep only non-finalized candidates for incremental advancement.
    while observed_checkpoints
        .front()
        .is_some_and(|ckpt| finalized_epoch.is_some_and(|fin| ckpt.tip.epoch <= fin.epoch()))
    {
        observed_checkpoints.pop_front();
    }

    Ok(DerivedState {
        observed_checkpoints,
        new_clstate,
    })
}

/// Builds the initial L1 block list from the reorg-safe finalized floor up to
/// `cur_block`.
fn init_recent_processed_blocks(
    ctx: &impl CsmWorkerContext,
    cur_block: L1BlockCommitment,
) -> CsmWorkerResult<Vec<L1BlockCommitment>> {
    let depth = ctx.l1_reorg_safe_depth().max(1);
    let gen_height = ctx.genesis_l1_block().height();
    // Anchor is whichever is highest: depth from cur tip or genesis height
    let anchor_height = cur_block.height().saturating_sub(depth - 1).max(gen_height);

    let mut blocks = Vec::new();
    for height in anchor_height..=cur_block.height() {
        blocks.push(ctx.get_canonical_l1_block(height)?);
    }
    Ok(blocks)
}

/// Loads observed checkpoint candidates from the OL checkpoint DB via `ctx`,
/// starting from `start_epoch`.
fn load_observed_checkpoints<C: CsmWorkerContext>(
    ctx: &C,
    start_epoch: Epoch,
    current_l1_tip: L1Height,
) -> CsmWorkerResult<VecDeque<L1Checkpoint>> {
    let Some(last_ep_with_l1ref) = ctx.get_last_checkpoint_l1_ref_epoch()? else {
        return Ok(VecDeque::new());
    };
    let last_checkpoint_epoch = last_ep_with_l1ref.epoch();

    let mut observed = VecDeque::new();
    for epoch in start_epoch..=last_checkpoint_epoch {
        let Some(commitment) = ctx.get_canonical_epoch_commitment_at(epoch)? else {
            continue;
        };
        let Some(observation) = ctx.get_checkpoint_l1_ref(commitment)? else {
            continue;
        };
        let Some(payload) = ctx.get_checkpoint_payload(commitment)? else {
            continue;
        };

        // Heights are strictly increasing in epoch order, so once one exceeds
        // the tip every later one does too.
        if observation.l1_commitment.height() > current_l1_tip {
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

    use super::CsmWorkerState;
    use crate::test_utils::StubCtx;

    fn create_test_params() -> Arc<AsmParams> {
        Arc::new(strata_test_utils_l2::gen_asm_params())
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

        let ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            4,
            params.magic,
            params.anchor.block,
        );
        let state = CsmWorkerState::bootstrap(ctx).expect("state init");

        assert_eq!(state.confirmed_epoch, Some(commitment_2));
        assert_eq!(state.finalized_epoch, Some(commitment_1));
        assert_eq!(state.observed_checkpoints.len(), 1);
        assert_eq!(
            state
                .observed_checkpoints
                .front()
                .map(EpochCommitment::from),
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

        // Seed the on-disk ClientState so its `last_finalized_checkpoint`
        // already reflects epoch 1 — bootstrap should observe this and skip
        // the refresh path.
        let baseline = ClientState::new(
            Some(L1Checkpoint::new(*payload_1.new_tip(), l1_ref_1)),
            None,
        );
        let baseline_block = L1BlockCommitment::new(20, L1BlockId::default());
        storage
            .client_state()
            .put_update_blocking(
                &baseline_block,
                ClientUpdateOutput::new(baseline.clone(), vec![]),
            )
            .expect("seed baseline client state");

        let ctx = StubCtx::new(
            storage.clone(),
            status_channel,
            4,
            params.magic,
            params.anchor.block,
        );
        let state = CsmWorkerState::bootstrap(ctx).expect("state init");

        assert_eq!(state.finalized_epoch, Some(commitment_1));
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
}
