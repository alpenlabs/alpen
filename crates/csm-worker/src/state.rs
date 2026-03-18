//! CSM worker service state.

use std::{collections::VecDeque, sync::Arc};

use strata_csm_types::ClientState;
use strata_db_types::types::OLCheckpointL1ObservationEntry;
use strata_identifiers::Epoch;
use strata_params::Params;
use strata_primitives::prelude::*;
use strata_service::ServiceState;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

use crate::constants;

/// State for the CSM worker service.
///
/// This state is used by the CSM worker which acts as a listener to ASM worker
/// status updates, processing checkpoint logs from the checkpoint-v0 subprotocol.
#[expect(
    missing_debug_implementations,
    reason = "NodeStorage doesn't implement Debug"
)]
pub struct CsmWorkerState {
    /// Consensus parameters.
    pub(crate) params: Arc<Params>,

    /// Node storage handle.
    pub(crate) storage: Arc<NodeStorage>,

    /// Current client state.
    pub(crate) cur_state: Arc<ClientState>,

    /// Last ASM update we processed.
    pub(crate) last_asm_block: Option<L1BlockCommitment>,

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
    pub(crate) observed_checkpoints: VecDeque<(EpochCommitment, OLCheckpointL1ObservationEntry)>,

    /// Status channel for publishing state updates.
    pub(crate) status_channel: Arc<StatusChannel>,
}

impl CsmWorkerState {
    /// Create a new CSM worker state.
    pub fn new(
        params: Arc<Params>,
        storage: Arc<NodeStorage>,
        status_channel: Arc<StatusChannel>,
    ) -> anyhow::Result<Self> {
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

        let finalized_from_l1_observations =
            derive_finalized_epoch(observed_checkpoints.iter(), current_l1_tip, finality_depth);
        let finalized_epoch =
            max_epoch_commitment(baseline_finalized_epoch, finalized_from_l1_observations);

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
            params,
            storage,
            cur_state: Arc::new(cur_state),
            last_asm_block: Some(cur_block),
            last_processed_epoch: None,
            confirmed_epoch,
            finalized_epoch,
            observed_checkpoints,
            status_channel,
        })
    }

    /// Get the last ASM block that was processed.
    pub fn last_asm_block(&self) -> Option<L1BlockCommitment> {
        self.last_asm_block
    }
}

/// Loads observed checkpoint candidates from the OL checkpoint DB starting from `start_epoch`.
///
/// Only epochs with both a canonical commitment and an L1 observation entry are included.
/// Used at startup to populate the incremental finalization queue.
fn load_observed_checkpoints_from_db(
    storage: &NodeStorage,
    start_epoch: Epoch,
    current_l1_tip: L1Height,
) -> anyhow::Result<VecDeque<(EpochCommitment, OLCheckpointL1ObservationEntry)>> {
    let ol_checkpoint = storage.ol_checkpoint();
    let Some(last_payload_commitment) =
        ol_checkpoint.get_last_checkpoint_payload_epoch_blocking()?
    else {
        return Ok(VecDeque::new());
    };
    let last_checkpoint_epoch = last_payload_commitment.epoch();

    let mut observed = VecDeque::new();
    for epoch in start_epoch..=last_checkpoint_epoch {
        let Some(commitment) = ol_checkpoint.get_canonical_epoch_commitment_at_blocking(epoch)?
        else {
            continue;
        };
        let Some(observation) =
            ol_checkpoint.get_checkpoint_l1_observation_entry_blocking(commitment)?
        else {
            continue;
        };

        if observation.l1_block.height() > current_l1_tip {
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
    I: Iterator<Item = &'a (EpochCommitment, OLCheckpointL1ObservationEntry)>,
{
    let mut latest_finalized = None;
    let finality_depth = finality_depth.max(1);

    for (commitment, observation) in observed {
        let confirmations = current_l1_tip
            .saturating_sub(observation.l1_block.height())
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

impl ServiceState for CsmWorkerState {
    fn name(&self) -> &str {
        constants::SERVICE_NAME
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_checkpoint_types::EpochSummary;
    use strata_checkpoint_types_ssz::test_utils::create_test_checkpoint_payload;
    use strata_csm_types::{ClientState, ClientUpdateOutput};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_db_types::types::OLCheckpointL1ObservationEntry;
    use strata_identifiers::Buf32;
    use strata_params::{Params, RollupParams, SyncParams};
    use strata_primitives::prelude::*;
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
    use strata_test_utils::ArbitraryGenerator;

    use super::CsmWorkerState;

    fn create_test_params() -> Arc<Params> {
        let params_json = r#"{
            "magic_bytes": "ALPN",
            "block_time": 1000,
            "cred_rule": {
                "schnorr_key": "c18d86b16f91b01a6599c3a290c1f255784f89dfe31ea65f64c4bdbd01564873"
            },
            "genesis_l1_view": {
                "blk": {
                    "height": 100,
                    "blkid": "a99c81cc79d156fda27bf222537ce1de784921a52730df60ead99404b43f622a"
                },
                "next_target": 545259519,
                "epoch_start_timestamp": 1296688602,
                "last_11_timestamps": [
                    1760287556, 1760287556, 1760287557, 1760287557, 1760287557,
                    1760287557, 1760287557, 1760287557, 1760287558, 1760287558, 1760287558
                ]
            },
            "operators": [
                "6e31167a21a20186c270091f3705ba9ba0f9649af9281a4331962a2f02f0b382",
                "59df7b48d6adbb11fb9f8e4d4a296df83b3edcff6573e80b6c77cdcc4a729ecc",
                "9ac5088dcf5dea3593e6095250875c89a0138b3e027f615d782be2080a5e4bac",
                "f86435262dde652b3aef97a4a8cc9ae19aa5da13159e778da0fbceb3a3adb923"
            ],
            "evm_genesis_block_hash": "46c0dc60fb131be4ccc55306a345fcc20e44233324950f978ba5f185aa2af4dc",
            "evm_genesis_block_state_root": "351714af72d74259f45cd7eab0b04527cd40e74836a45abcae50f92d919d988f",
            "l1_reorg_safe_depth": 4,
            "target_l2_batch_size": 64,
            "deposit_amount": 1000000000,
            "recovery_delay": 1008,
            "checkpoint_predicate": "AlwaysAccept",
            "dispatch_assignment_dur": 64,
            "proof_publish_mode": {
                "timeout": 1
            },
            "max_deposits_in_block": 16,
            "network": "signet"
        }"#;

        let rollup_params: RollupParams =
            serde_json::from_str(params_json).expect("Failed to parse test params");
        Arc::new(Params {
            rollup: rollup_params,
            run: SyncParams {
                l1_follow_distance: 10,
                client_checkpoint_interval: 100,
                l2_blocks_fetch_limit: 1000,
            },
        })
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
            .put_checkpoint_payload_entry_blocking(commitment_1, payload_1)
            .expect("insert epoch 1 payload");
        ol_checkpoint
            .put_checkpoint_l1_observation_entry_blocking(
                commitment_1,
                OLCheckpointL1ObservationEntry::new(L1BlockCommitment::new(
                    17,
                    L1BlockId::default(),
                )),
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
            .put_checkpoint_payload_entry_blocking(commitment_2, payload_2)
            .expect("insert epoch 2 payload");
        ol_checkpoint
            .put_checkpoint_l1_observation_entry_blocking(
                commitment_2,
                OLCheckpointL1ObservationEntry::new(L1BlockCommitment::new(
                    19,
                    L1BlockId::default(),
                )),
            )
            .expect("insert epoch 2 observation");

        let state = CsmWorkerState::new(params, storage, status_channel).expect("state init");

        assert_eq!(state.confirmed_epoch, Some(commitment_2));
        assert_eq!(state.finalized_epoch, Some(commitment_1));
        assert_eq!(state.observed_checkpoints.len(), 1);
        assert_eq!(
            state.observed_checkpoints.front().map(|(epoch, _)| *epoch),
            Some(commitment_2)
        );
    }
}
