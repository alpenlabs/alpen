//! CSM worker service state.

use std::{collections::VecDeque, sync::Arc};

use strata_csm_types::{CheckpointL1Ref, ClientState};
use strata_identifiers::Epoch;
use strata_params::Params;
use strata_primitives::prelude::*;
use strata_service::ServiceState;
use strata_storage::NodeStorage;

use crate::{constants, context::CsmWorkerContext};

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

    /// Last ASM block committed. Advanced only by a successful commit; never
    /// rolled back.
    pub(crate) last_asm_block: Option<L1BlockCommitment>,

    /// State staged while processing the current ASM block's logs; committed as
    /// a unit by `commit_block` or rolled back on any failure.
    pub(crate) staged: StagedState,
}

/// In-memory state that is provisionally mutated while processing an ASM
/// block's logs.
///
/// An ASM block is processed and committed as a unit: every field here is
/// snapshotted before processing the block's logs and restored on any failure,
/// so a retry replays from the last committed baseline.
#[derive(Clone)]
pub(crate) struct StagedState {
    /// Current client state.
    pub(crate) cur_state: Arc<ClientState>,

    /// Last epoch we processed a checkpoint for.
    pub(crate) last_processed_epoch: Option<Epoch>,

    /// Latest checkpoint epoch observed on L1.
    pub(crate) confirmed_epoch: Option<EpochCommitment>,

    /// Latest checkpoint epoch finalized by L1 depth, derived from observation facts and tip.
    pub(crate) finalized_epoch: Option<EpochCommitment>,

    /// Ordered observed checkpoint candidates used for incremental depth derivation.
    ///
    /// Items are appended when new observation facts are written and consumed as
    /// finalized depth progresses.
    pub(crate) observed_checkpoints: VecDeque<(EpochCommitment, CheckpointL1Ref)>,
}

impl<C: CsmWorkerContext> CsmWorkerState<C> {
    /// Create a new CSM worker state.
    ///
    /// `params` and `storage` are read once at startup to bootstrap the
    /// in-memory finalization queue; runtime persistence and L1 fetches go
    /// through `ctx`.
    pub fn new(params: Arc<Params>, storage: Arc<NodeStorage>, ctx: C) -> anyhow::Result<Self> {
        // Load the most recent client state from storage
        let (cur_block, cur_state) = storage
            .client_state()
            .fetch_most_recent_state()?
            .unwrap_or((params.rollup.genesis_l1_view.blk, ClientState::default()));

        let current_l1_tip = cur_block.height();
        let finality_depth = params.rollup.l1_reorg_safe_depth.max(1);
        let baseline_finalized_epoch = cur_state.get_declared_final_epoch();
        let observation_start_epoch = baseline_finalized_epoch
            .map(|epoch| epoch.epoch().saturating_add(1))
            .unwrap_or(0);

        let mut observed_checkpoints = load_observed_checkpoints_from_db(
            storage.as_ref(),
            observation_start_epoch,
            current_l1_tip,
        )?;

        let finalized_from_l1_refs =
            derive_finalized_epoch(observed_checkpoints.iter(), current_l1_tip, finality_depth);
        let finalized_epoch =
            max_epoch_commitment(baseline_finalized_epoch, finalized_from_l1_refs);

        // Confirmed means "observed on L1" and may be finalized. If we only loaded
        // observations after finalized, fall back to finalized when no newer observed entry exists.
        let confirmed_epoch = observed_checkpoints
            .back()
            .map(|(epoch, _)| *epoch)
            .or(finalized_epoch);

        // Keep only non-finalized candidates for incremental tip-driven advancement.
        if let Some(finalized) = finalized_epoch {
            while observed_checkpoints
                .front()
                .is_some_and(|(epoch, _)| epoch.epoch() <= finalized.epoch())
            {
                observed_checkpoints.pop_front();
            }
        }

        Ok(Self {
            ctx,
            last_asm_block: Some(cur_block),
            staged: StagedState {
                cur_state: Arc::new(cur_state),
                last_processed_epoch: None,
                confirmed_epoch,
                finalized_epoch,
                observed_checkpoints,
            },
        })
    }

    /// Get the last ASM block that was processed.
    pub fn get_last_asm_block(&self) -> Option<L1BlockCommitment> {
        self.last_asm_block
    }
}

/// Loads observed checkpoint candidates from the OL checkpoint DB starting from `start_epoch`.
///
/// Only epochs with both a canonical commitment and an L1 ref are included.
/// Used at startup to populate the incremental finalization queue.
fn load_observed_checkpoints_from_db(
    storage: &NodeStorage,
    start_epoch: Epoch,
    current_l1_tip: L1Height,
) -> anyhow::Result<VecDeque<(EpochCommitment, CheckpointL1Ref)>> {
    let ol_checkpoint = storage.ol_checkpoint();
    let Some(last_l1_ref_commitment) = ol_checkpoint.get_last_checkpoint_l1_ref_epoch_blocking()?
    else {
        return Ok(VecDeque::new());
    };
    let last_checkpoint_epoch = last_l1_ref_commitment.epoch();

    let mut observed = VecDeque::new();
    for epoch in start_epoch..=last_checkpoint_epoch {
        let Some(commitment) = ol_checkpoint.get_canonical_epoch_commitment_at_blocking(epoch)?
        else {
            continue;
        };
        let Some(observation) = ol_checkpoint.get_checkpoint_l1_ref_blocking(commitment)? else {
            continue;
        };

        if observation.l1_commitment.height() > current_l1_tip {
            continue;
        }

        observed.push_back((commitment, observation));
    }

    Ok(observed)
}

/// Returns the latest epoch commitment whose observation meets the depth threshold.
///
/// Iterates forward; the last match wins (latest finalized).
fn derive_finalized_epoch<'a, I>(
    observed: I,
    current_l1_tip: L1Height,
    finality_depth: u32,
) -> Option<EpochCommitment>
where
    I: Iterator<Item = &'a (EpochCommitment, CheckpointL1Ref)>,
{
    let mut latest_finalized = None;
    let finality_depth = finality_depth.max(1);

    for (commitment, observation) in observed {
        let confirmations = current_l1_tip
            .saturating_sub(observation.l1_commitment.height())
            .saturating_add(1);
        if confirmations >= finality_depth {
            latest_finalized = Some(*commitment);
        }
    }

    latest_finalized
}

/// Returns the epoch commitment with the higher epoch number, or whichever is `Some`.
fn max_epoch_commitment(
    left: Option<EpochCommitment>,
    right: Option<EpochCommitment>,
) -> Option<EpochCommitment> {
    match (left, right) {
        (Some(a), Some(b)) => Some(if a.epoch() >= b.epoch() { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

impl<C: CsmWorkerContext + 'static> ServiceState for CsmWorkerState<C> {
    fn name(&self) -> &str {
        constants::SERVICE_NAME
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_asm_proto_checkpoint_types::test_utils::create_test_checkpoint_payload;
    use strata_checkpoint_types::EpochSummary;
    use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::{Buf32, L1BlockId, RBuf32};
    use strata_params::Params;
    use strata_primitives::prelude::*;
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
    use strata_test_utils::ArbitraryGenerator;

    use super::CsmWorkerState;
    use crate::test_utils::StubCtx;

    fn create_test_params() -> Arc<Params> {
        Arc::new(strata_test_utils_l2::gen_params())
    }

    fn create_test_storage_and_status(
        params: Arc<Params>,
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
            params.rollup.genesis_l1_view.blk,
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
            params.rollup.magic_bytes,
        );
        let state = CsmWorkerState::new(params, storage, ctx).expect("state init");

        assert_eq!(state.staged.confirmed_epoch, Some(commitment_2));
        assert_eq!(state.staged.finalized_epoch, Some(commitment_1));
        assert_eq!(state.staged.observed_checkpoints.len(), 1);
        assert_eq!(
            state
                .staged
                .observed_checkpoints
                .front()
                .map(|(epoch, _)| *epoch),
            Some(commitment_2)
        );
    }
}
