//! Shared checkpoint processing logic used by both v0 and v1 processors.

use std::sync::Arc;

use strata_csm_types::{ClientState, ClientUpdateOutput, L1Checkpoint, SyncAction};
use strata_identifiers::Epoch;
use tracing::*;

use crate::{state::CsmWorkerState, sync_actions::apply_action};

/// Update client state with a new checkpoint.
pub(crate) fn update_client_state_with_checkpoint(
    state: &mut CsmWorkerState,
    new_checkpoint: L1Checkpoint,
    epoch: Epoch,
) -> anyhow::Result<()> {
    // Get the current client state
    let cur_state = state.cur_state.as_ref();

    // Determine if this checkpoint should be the last finalized or just recent.

    // TODO(STR-2438): This comes from the legacy design currently and will be
    // simplified in the future.
    // Currently, `last_finalized` is the buried checkpoint and recent and the last be observed (the
    // checkpoint that makes the the finalized one to be buried).

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

    // Create new client state
    let next_state = ClientState::new(last_finalized, recent.clone());

    // Check if we need to finalize an epoch
    let old_final_epoch = cur_state.get_declared_final_epoch();
    let new_final_epoch = next_state.get_declared_final_epoch();

    let should_finalize = match (old_final_epoch, new_final_epoch) {
        (None, Some(new)) => Some(new),
        (Some(old), Some(new)) if new.epoch() > old.epoch() => Some(new),
        _ => None,
    };

    // Store the new client state
    let l1_block = state.last_asm_block.expect("should have ASM block");
    state.storage.client_state().put_update_blocking(
        &l1_block,
        ClientUpdateOutput::new(next_state.clone(), vec![]),
    )?;

    // Update our tracked state
    state.cur_state = Arc::new(next_state);

    // Update status channel
    state
        .status_channel
        .update_client_state(state.cur_state.as_ref().clone(), l1_block);

    // Handle epoch finalization if needed
    if let Some(epoch_comm) = should_finalize {
        info!(?epoch_comm, "Finalizing epoch from checkpoint");
        apply_action(SyncAction::FinalizeEpoch(epoch_comm), &state.storage)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_checkpoint_types::BatchInfo;
    use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_params::{Params, RollupParams, SyncParams};
    use strata_primitives::{
        buf::Buf32,
        l2::{L2BlockCommitment, L2BlockId},
        prelude::*,
    };
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
    use strata_test_utils::ArbitraryGenerator;

    use super::update_client_state_with_checkpoint;
    use crate::state::CsmWorkerState;

    /// Helper to create a test CSM worker state.
    fn create_test_state() -> (CsmWorkerState, Arc<strata_storage::NodeStorage>) {
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

        let params = Params {
            rollup: rollup_params,
            run: SyncParams {
                l1_follow_distance: 10,
                client_checkpoint_interval: 100,
                l2_blocks_fetch_limit: 1000,
            },
        };
        let params = Arc::new(params);

        // Create an in-memory database for testing
        let db = get_test_sled_backend();
        let pool = threadpool::ThreadPool::new(4);

        let storage = Arc::new(create_node_storage(db, pool).expect("Failed to create storage"));

        // Initialize with empty client state
        let initial_state = ClientState::new(None, None);
        let initial_block = L1BlockCommitment::new(0, L1BlockId::default());

        storage
            .client_state()
            .put_update_blocking(
                &initial_block,
                ClientUpdateOutput::new(initial_state.clone(), vec![]),
            )
            .expect("Failed to initialize client state");

        // Create status channel with proper arguments
        let mut arbgen = ArbitraryGenerator::new();
        let status_channel = StatusChannel::new(
            arbgen.generate(),
            arbgen.generate(),
            arbgen.generate(),
            None,
            None,
        );

        let state =
            CsmWorkerState::new(params.clone(), storage.clone(), status_channel.into()).unwrap();

        (state, storage)
    }

    /// Helper to build a minimal [`L1Checkpoint`] for testing.
    fn make_checkpoint(
        epoch: u32,
        asm_block: &L1BlockCommitment,
        arbgen: &mut ArbitraryGenerator,
    ) -> L1Checkpoint {
        let l2_start = L2BlockCommitment::new(
            ((epoch - 1) * 10) as u64,
            L2BlockId::from(Buf32::from([epoch as u8; 32])),
        );
        let l2_end = L2BlockCommitment::new(
            (epoch * 10) as u64,
            L2BlockId::from(Buf32::from([(epoch + 1) as u8; 32])),
        );
        let l1_start = L1BlockCommitment::new(90 + epoch - 1, arbgen.generate());
        let l1_end = L1BlockCommitment::new(90 + epoch, arbgen.generate());
        let batch_info = BatchInfo::new(epoch, (l1_start, l1_end), (l2_start, l2_end));
        let l1_ref = CheckpointL1Ref::new(*asm_block, Buf32::zero(), Buf32::zero());
        L1Checkpoint::new(batch_info, l1_ref)
    }

    #[test]
    fn test_sequential_checkpoint_updates_finalize_epoch() {
        let (mut state, _) = create_test_state();
        let mut arbgen = ArbitraryGenerator::new();

        for epoch in 1u32..=2u32 {
            let asm_block = L1BlockCommitment::new(200 + epoch, arbgen.generate());
            state.last_asm_block = Some(asm_block);

            let checkpoint = make_checkpoint(epoch, &asm_block, &mut arbgen);
            let result = update_client_state_with_checkpoint(&mut state, checkpoint, epoch);
            assert!(
                result.is_ok(),
                "update should succeed for epoch {epoch}: {result:?}",
            );
        }

        // After two sequential checkpoints, epoch 1 should be declared finalized.
        let declared_final_epoch = state
            .cur_state
            .as_ref()
            .get_declared_final_epoch()
            .expect("expected finalized epoch after two updates");
        assert_eq!(declared_final_epoch.epoch(), 1);
    }
}
