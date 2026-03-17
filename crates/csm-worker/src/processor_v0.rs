//! Processor for Checkpoint V0

use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{CheckpointUpdate, constants::CHECKPOINT_UPDATE_LOG_TYPE};
use strata_checkpoint_types::{Checkpoint, CheckpointSidecar};
use strata_csm_types::{CheckpointL1Ref, L1Checkpoint, SyncAction};
use strata_primitives::prelude::*;
use tracing::*;

use crate::{
    processor::update_client_state_with_checkpoint, state::CsmWorkerState,
    sync_actions::apply_action,
};

pub(crate) fn handle_checkpoint_v0_updates(
    state: &mut CsmWorkerState,
    asm_block: &L1BlockCommitment,
    logs: &[AsmLogEntry],
) -> anyhow::Result<()> {
    // Filter logs for checkpoint updates
    let checkpoint_logs: Vec<&AsmLogEntry> = logs
        .iter()
        .filter(|log| log.ty() == Some(CHECKPOINT_UPDATE_LOG_TYPE))
        .collect();

    if checkpoint_logs.is_empty() {
        trace!(%asm_block, "No checkpoint update logs in ASM status update");
        return Ok(());
    }

    let logs_num = checkpoint_logs.len();
    trace!(%logs_num, %asm_block, "CSM received checkpoint update logs from ASM");

    for log in checkpoint_logs {
        let ckpt_upd: CheckpointUpdate = log
            .try_into_log()
            .map_err(|e| anyhow::anyhow!("Failed to deserialize CheckpointUpdate: {}", e))?;
        process_checkpoint_log(state, &ckpt_upd, asm_block)?;
    }

    Ok(())
}

/// Process a single ASM log entry, extracting and handling checkpoint updates.
fn process_checkpoint_log(
    state: &mut CsmWorkerState,
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
    let l1_checkpoint =
        L1Checkpoint::new(checkpoint_update.batch_info().clone(), l1_reference.clone());

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

/// Create a [`Checkpoint`] from a [`CheckpointUpdate`] log.
///
/// Note: The log doesn't contain the full signed checkpoint, so we reconstruct
/// what we can. The signature verification was already done by ASM.
// TODO(STR-2438): This function exists for compatibility to avoid larger changes.
// It should be reworked for the new OL STF where checkpoint structures differ.
fn create_checkpoint_from_update(update: &CheckpointUpdate) -> Checkpoint {
    // Create empty sidecar - checkpoint was already verified by ASM
    let sidecar = CheckpointSidecar::new(vec![]);

    Checkpoint::new(
        update.batch_info().clone(),
        Default::default(), // Empty proof - actual proof was already verified by ASM
        sidecar,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_asm_common::AsmLogEntry;
    use strata_asm_logs::{CheckpointUpdate, constants::CHECKPOINT_UPDATE_LOG_TYPE};
    use strata_checkpoint_types::BatchInfo;
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

    use super::handle_checkpoint_v0_updates;
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

        let db = get_test_sled_backend();
        let pool = threadpool::ThreadPool::new(4);

        let storage = Arc::new(create_node_storage(db, pool).expect("Failed to create storage"));

        let initial_state = ClientState::new(None, None);
        let initial_block = L1BlockCommitment::new(0, L1BlockId::default());

        storage
            .client_state()
            .put_update_blocking(
                &initial_block,
                ClientUpdateOutput::new(initial_state.clone(), vec![]),
            )
            .expect("Failed to initialize client state");

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

    #[test]
    fn test_unknown_log_type_is_ignored() {
        let (mut state, _) = create_test_state();
        let asm_block = L1BlockCommitment::new(100, L1BlockId::default());

        let log = AsmLogEntry::from_msg(999, vec![1, 2, 3, 4]).expect("Failed to create log");

        let result = handle_checkpoint_v0_updates(&mut state, &asm_block, &[log]);
        assert!(result.is_ok(), "should handle unknown log types gracefully");
        assert_eq!(state.last_processed_epoch, None);
    }

    #[test]
    fn test_typeless_log_is_ignored() {
        let (mut state, _) = create_test_state();
        let asm_block = L1BlockCommitment::new(100, L1BlockId::default());

        let log = AsmLogEntry::from_raw(vec![5, 6, 7, 8]);

        let result = handle_checkpoint_v0_updates(&mut state, &asm_block, &[log]);
        assert!(result.is_ok(), "should handle typeless logs gracefully");
        assert_eq!(state.last_processed_epoch, None);
    }

    #[test]
    fn test_invalid_checkpoint_data_returns_error() {
        let (mut state, _) = create_test_state();
        let asm_block = L1BlockCommitment::new(100, L1BlockId::default());
        state.last_asm_block = Some(asm_block);

        let invalid_log = AsmLogEntry::from_msg(CHECKPOINT_UPDATE_LOG_TYPE, vec![1, 2, 3])
            .expect("Failed to create log");

        let result = handle_checkpoint_v0_updates(&mut state, &asm_block, &[invalid_log]);
        assert!(result.is_err(), "should fail with invalid checkpoint data");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to deserialize CheckpointUpdate"),
            "Error should mention deserialization failure"
        );
    }

    #[test]
    fn test_sequential_checkpoint_logs_happy_path() {
        let (mut state, storage) = create_test_state();
        let mut arbgen = ArbitraryGenerator::new();

        for epoch in 1u32..=3u32 {
            let asm_block = L1BlockCommitment::new(100 + epoch, arbgen.generate());
            state.last_asm_block = Some(asm_block);

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
            let epoch_commitment = EpochCommitment::from_terminal(epoch, l2_end);
            let checkpoint_txid: BitcoinTxid = arbgen.generate();

            let checkpoint_update =
                CheckpointUpdate::new(epoch_commitment, batch_info, checkpoint_txid);
            let log = AsmLogEntry::from_log(&checkpoint_update).expect("make log");

            let result = handle_checkpoint_v0_updates(&mut state, &asm_block, &[log]);
            assert!(
                result.is_ok(),
                "should succeed for epoch {epoch}: {result:?}",
            );

            assert_eq!(
                state.last_processed_epoch,
                Some(epoch),
                "Last processed epoch should be updated to {epoch}",
            );

            #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
            let stored_checkpoint = storage
                .checkpoint()
                .get_checkpoint_blocking(epoch as u64)
                .expect("Failed to query checkpoint database");
            assert!(
                stored_checkpoint.is_some(),
                "Checkpoint for epoch {epoch} should be stored in database",
            );

            let last_checkpoint = state.cur_state.as_ref().get_last_checkpoint();
            assert!(
                last_checkpoint.is_some(),
                "Client state should have a last checkpoint after epoch {epoch}",
            );
            assert_eq!(
                last_checkpoint.unwrap().batch_info.epoch(),
                epoch,
                "Last checkpoint in client state should be for epoch {epoch}",
            );
        }

        // Verify all checkpoints are still in the database
        for epoch in 1u32..=3u32 {
            #[expect(deprecated, reason = "legacy old code is retained for compatibility")]
            let stored_checkpoint = storage
                .checkpoint()
                .get_checkpoint_blocking(epoch as u64)
                .expect("Failed to query checkpoint database");
            assert!(
                stored_checkpoint.is_some(),
                "Checkpoint for epoch {epoch} should still be in database",
            );
        }
    }
}
