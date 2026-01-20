//! Checkpoint log processing logic.

use std::{fmt::Debug, sync::Arc};

use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{CheckpointUpdate, constants::CHECKPOINT_UPDATE_LOG_TYPE};
use strata_checkpoint_types::{BatchTransition, Checkpoint, CheckpointSidecar};
use strata_csm_types::{
    CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint, SyncAction,
};
use strata_identifiers::Epoch;
use strata_primitives::prelude::*;
use tracing::*;

use crate::{state::CsmWorkerState, sync_actions::apply_action};

pub(crate) fn process_log<State: Clone + Debug + Send + Sync + 'static>(
    state: &mut CsmWorkerState<State>,
    log: &AsmLogEntry,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    match log.ty() {
        Some(CHECKPOINT_UPDATE_LOG_TYPE) => {
            let ckpt_upd = log
                .try_into_log()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize CheckpointUpdate: {}", e))?;

            return process_checkpoint_log(state, &ckpt_upd, asm_block);
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

/// Process a single ASM log entry, extracting and handling checkpoint updates.
fn process_checkpoint_log<State: Clone + Debug + Send + Sync + 'static>(
    state: &mut CsmWorkerState<State>,
    checkpoint_update: &CheckpointUpdate,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    let epoch = checkpoint_update.batch_info().epoch();

    info!(
        %epoch,
        %asm_block,
        checkpoint_txid = ?checkpoint_update.checkpoint_txid(),
        "CSM is processing checkpoint update from ASM log"
    );

    // Create L1 checkpoint reference from the log data
    let l1_reference = CheckpointL1Ref::new(
        *asm_block,
        checkpoint_update.checkpoint_txid().inner_raw(),
        checkpoint_update.checkpoint_txid().inner_raw(), // TODO: get wtxid if available
    );

    // Create L1Checkpoint for client state
    let l1_checkpoint = L1Checkpoint::new(
        checkpoint_update.batch_info().clone(),
        BatchTransition {
            epoch,
            chainstate_transition: *checkpoint_update.chainstate_transition(),
        },
        l1_reference.clone(),
    );

    // Update the client state with this checkpoint
    update_client_state_with_checkpoint(state, l1_checkpoint, epoch)?;

    // Create sync action to update checkpoint entry in database
    let sync_action = SyncAction::UpdateCheckpointInclusion {
        checkpoint: create_checkpoint_from_update(checkpoint_update),
        l1_reference,
    };

    // Apply the sync action
    apply_action(sync_action, &state.storage)?;

    // Track the last processed epoch
    state.last_processed_epoch = Some(epoch);

    Ok(())
}

/// Update client state with a new checkpoint.
fn update_client_state_with_checkpoint<State: Clone + Debug + Send + Sync + 'static>(
    state: &mut CsmWorkerState<State>,
    new_checkpoint: L1Checkpoint,
    epoch: Epoch,
) -> anyhow::Result<()> {
    // Get the current client state
    let cur_state = state.cur_state.as_ref();

    // Determine if this checkpoint should be the last finalized or just recent.

    // TODO: This comes from the legacy design currently and will be simplified in the future.
    // Currently, `last_finalized` is the buried checkpoint and recent and the last be observed (the
    // checkpoint that makes the the finalized one to be buried).

    // TODO: it's better to store `L1Checkpoint` separately, move the logic of "recent/finalized"
    // to the DbManager (that can actually fetches actual persisted data and doesn't rely on the
    // current state).
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

/// Create a [`Checkpoint`] from a [`CheckpointUpdate`] log.
///
/// Note: The log doesn't contain the full signed checkpoint, so we reconstruct
/// what we can. The signature verification was already done by ASM.
///
/// TODO: This function is created for compatibility reason to avoid making larger changes.
/// This will be largely changed as we move to the new OL STF as the checkpoint structure
/// will be different than the existing ones.
fn create_checkpoint_from_update(update: &CheckpointUpdate) -> Checkpoint {
    let epoch = update.batch_info().epoch();

    // Create empty sidecar - checkpoint was already verified by ASM
    let sidecar = CheckpointSidecar::new(vec![]);

    Checkpoint::new(
        update.batch_info().clone(),
        BatchTransition {
            epoch,
            chainstate_transition: *update.chainstate_transition(),
        },
        Default::default(), // Empty proof - actual proof was already verified by ASM
        sidecar,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bitcoin::absolute::Height;
    use strata_asm_common::AsmLogEntry;
    use strata_asm_logs::{CheckpointUpdate, constants::CHECKPOINT_UPDATE_LOG_TYPE};
    use strata_checkpoint_types::{BatchInfo, ChainstateRootTransition};
    use strata_csm_types::{ClientState, ClientUpdateOutput};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_params::{Params, RollupParams, SyncParams};
    use strata_primitives::{
        buf::Buf32,
        epoch::EpochCommitment,
        l1::BitcoinTxid,
        l2::{L2BlockCommitment, L2BlockId},
        prelude::*,
    };
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
    use strata_test_utils::ArbitraryGenerator;

    use super::process_log;
    use crate::state::CsmWorkerState;

    /// Helper to create a test CSM worker state
    fn create_test_state() -> (CsmWorkerState, Arc<strata_storage::NodeStorage>) {
        // rollup params (taken from a fntests run).
        // Don't we have some util fn for such?
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
            "operator_config": {
                "static": [
                    {
                        "signing_pk": "6e31167a21a20186c270091f3705ba9ba0f9649af9281a4331962a2f02f0b382",
                        "wallet_pk": "59df7b48d6adbb11fb9f8e4d4a296df83b3edcff6573e80b6c77cdcc4a729ecc"
                    },
                    {
                        "signing_pk": "9ac5088dcf5dea3593e6095250875c89a0138b3e027f615d782be2080a5e4bac",
                        "wallet_pk": "f86435262dde652b3aef97a4a8cc9ae19aa5da13159e778da0fbceb3a3adb923"
                    }
                ]
            },
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
        let initial_block = L1BlockCommitment::new(Height::ZERO, L1BlockId::default());

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
        );

        let state =
            CsmWorkerState::new(params.clone(), storage.clone(), status_channel.clone()).unwrap();

        (state, storage)
    }

    /// Helper to create an unknown log type entry
    fn create_unknown_log_type() -> AsmLogEntry {
        AsmLogEntry::from_msg(999, vec![1, 2, 3, 4]).expect("Failed to create log")
    }

    /// Helper to create a log entry without a type
    fn create_typeless_log() -> AsmLogEntry {
        AsmLogEntry::from_raw(vec![5, 6, 7, 8])
    }

    #[test]
    fn test_process_log_with_unknown_log_type() {
        let (mut state, _) = create_test_state();
        let asm_block =
            L1BlockCommitment::new(Height::from_consensus(100).unwrap(), L1BlockId::default());

        let log = create_unknown_log_type();

        // Should succeed but do nothing
        let result = process_log(&mut state, &log, &asm_block);
        assert!(result.is_ok(), "process_log should handle unknown types");

        // State should not be updated
        assert_eq!(state.last_processed_epoch, None);
    }

    #[test]
    fn test_process_log_with_no_log_type() {
        let (mut state, _) = create_test_state();
        let asm_block =
            L1BlockCommitment::new(Height::from_consensus(100).unwrap(), L1BlockId::default());

        let log = create_typeless_log();

        // Should succeed but do nothing
        let result = process_log(&mut state, &log, &asm_block);
        assert!(result.is_ok(), "process_log should handle typeless logs");

        // State should not be updated
        assert_eq!(state.last_processed_epoch, None);
    }

    #[test]
    fn test_process_log_with_invalid_checkpoint_data() {
        let (mut state, _) = create_test_state();
        let asm_block =
            L1BlockCommitment::new(Height::from_consensus(100).unwrap(), L1BlockId::default());
        state.last_asm_block = Some(asm_block);

        // Create a log with checkpoint type but invalid data
        let invalid_log = AsmLogEntry::from_msg(CHECKPOINT_UPDATE_LOG_TYPE, vec![1, 2, 3])
            .expect("Failed to create log");

        // Should fail with deserialization error
        let result = process_log(&mut state, &invalid_log, &asm_block);
        assert!(
            result.is_err(),
            "process_log should fail with invalid checkpoint data"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to deserialize CheckpointUpdate"),
            "Error should mention deserialization failure"
        );
    }

    #[test]
    fn test_process_sequential_checkpoint_logs_happy_path() {
        let (mut state, storage) = create_test_state();

        // Create 3 sequential checkpoints with increasing epochs
        let mut arbgen = ArbitraryGenerator::new();

        for epoch in 1u32..=3u32 {
            // Create L1 block commitment for this checkpoint
            let asm_block = L1BlockCommitment::new(
                Height::from_consensus(100 + epoch).unwrap(),
                arbgen.generate(),
            );
            state.last_asm_block = Some(asm_block);

            // Create L2 block range
            let l2_start = L2BlockCommitment::new(
                ((epoch - 1) * 10) as u64,
                L2BlockId::from(Buf32::from([epoch as u8; 32])),
            );
            let l2_end = L2BlockCommitment::new(
                (epoch * 10) as u64,
                L2BlockId::from(Buf32::from([(epoch + 1) as u8; 32])),
            );

            // Create L1 block range
            let l1_start = L1BlockCommitment::new(
                Height::from_consensus(90 + epoch - 1).unwrap(),
                arbgen.generate(),
            );
            let l1_end = L1BlockCommitment::new(
                Height::from_consensus(90 + epoch).unwrap(),
                arbgen.generate(),
            );

            // Create batch info
            let batch_info = BatchInfo::new(epoch, (l1_start, l1_end), (l2_start, l2_end));

            // Create epoch commitment
            let epoch_commitment = EpochCommitment::from_terminal(epoch, l2_end);

            // Create chainstate transition
            let chainstate_transition = ChainstateRootTransition {
                pre_state_root: Buf32::from([0u8; 32]),
                post_state_root: Buf32::from([epoch as u8; 32]),
            };

            // Create checkpoint txid
            let checkpoint_txid: BitcoinTxid = arbgen.generate();

            // Create CheckpointUpdate
            let checkpoint_update = CheckpointUpdate::new(
                epoch_commitment,
                batch_info,
                chainstate_transition,
                checkpoint_txid,
            );

            // Create log entry
            let log = AsmLogEntry::from_log(&checkpoint_update).expect("make log");

            // Process the log
            let result = process_log(&mut state, &log, &asm_block);
            assert!(
                result.is_ok(),
                "process_log should succeed for epoch {}: {:?}",
                epoch,
                result
            );

            // Verify state was updated
            assert_eq!(
                state.last_processed_epoch,
                Some(epoch),
                "Last processed epoch should be updated to {}",
                epoch
            );

            // Verify checkpoint was stored in database
            let stored_checkpoint = storage
                .checkpoint()
                .get_checkpoint_blocking(epoch as u64)
                .expect("Failed to query checkpoint database");
            assert!(
                stored_checkpoint.is_some(),
                "Checkpoint for epoch {} should be stored in database",
                epoch
            );

            // Verify client state was updated
            let current_client_state = state.cur_state.as_ref();
            let last_checkpoint = current_client_state.get_last_checkpoint();
            assert!(
                last_checkpoint.is_some(),
                "Client state should have a last checkpoint after epoch {}",
                epoch
            );
            assert_eq!(
                last_checkpoint.unwrap().batch_info.epoch(),
                epoch,
                "Last checkpoint in client state should be for epoch {}",
                epoch
            );
        }

        // After processing 3 checkpoints, verify we have all of them in the database
        for epoch in 1u32..=3u32 {
            let stored_checkpoint = storage
                .checkpoint()
                .get_checkpoint_blocking(epoch as u64)
                .expect("Failed to query checkpoint database");
            assert!(
                stored_checkpoint.is_some(),
                "Checkpoint for epoch {} should still be in database",
                epoch
            );
        }
    }
}
