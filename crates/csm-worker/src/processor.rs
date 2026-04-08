//! Checkpoint log processing logic.

use std::sync::Arc;

use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{constants::CHECKPOINT_TIP_UPDATE_LOG_TYPE, CheckpointTipUpdate};
use strata_checkpoint_types::{BatchInfo, Checkpoint, CheckpointSidecar};
use strata_csm_types::{
    CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint, SyncAction,
};
use strata_identifiers::Epoch;
use strata_primitives::prelude::*;
use tracing::*;

use crate::{state::CsmWorkerState, sync_actions::apply_action};

pub(crate) fn process_log(
    state: &mut CsmWorkerState,
    log: &AsmLogEntry,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    match log.ty() {
        Some(CHECKPOINT_TIP_UPDATE_LOG_TYPE) => {
            let tip_upd = log
                .try_into_log()
                .map_err(|e| anyhow::anyhow!("Failed to deserialize CheckpointTipUpdate: {}", e))?;

            return process_checkpoint_tip_log(state, &tip_upd, asm_block);
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

/// Process a checkpoint tip update log from the v1 checkpoint subprotocol.
fn process_checkpoint_tip_log(
    state: &mut CsmWorkerState,
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

    // v1 tip logs do not contain full batch transition details.
    // CSM only needs epoch progression for finalized-epoch signaling, so we
    // synthesize a minimal checkpoint view from the tip while preserving
    // txid/wtxid from the tip update log.
    // TODO(STR-2438): Remove this synthetic mapping once CSM persists/consumes
    // checkpoint-v1-native fields without legacy L1Checkpoint shape coupling.
    let synthetic_checkpoint = checkpoint_from_tip_update(checkpoint_tip_update, asm_block);
    update_client_state_with_checkpoint(state, synthetic_checkpoint, epoch)?;
    mark_ol_checkpoint_l1_observed(state, checkpoint_tip_update, asm_block)?;

    state.last_processed_epoch = Some(epoch);
    Ok(())
}

/// Update and persist client state from a checkpoint.
fn update_client_state_with_checkpoint(
    state: &mut CsmWorkerState,
    new_checkpoint: L1Checkpoint,
    epoch: Epoch,
) -> anyhow::Result<()> {
    // Get the current client state.
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

    // Create new client state.
    let next_state = ClientState::new(last_finalized, recent);

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

    Ok(())
}

fn mark_ol_checkpoint_l1_observed(
    state: &mut CsmWorkerState,
    checkpoint_tip_update: &CheckpointTipUpdate,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    let tip = checkpoint_tip_update.tip();
    let _span = info_span!("mark_ol_checkpoint_l1_observed", epoch = tip.epoch).entered();
    let commitment = EpochCommitment::from_terminal(tip.epoch, *tip.l2_commitment());
    let ol_checkpoint = state.storage.ol_checkpoint();
    let checkpoint_txid = *checkpoint_tip_update.checkpoint_txid();
    // TODO(STR-2952): populate real checkpoint wtxid here.
    let checkpoint_wtxid = checkpoint_txid;

    let observation = CheckpointL1Ref::new(*asm_block, checkpoint_txid, checkpoint_wtxid);
    ol_checkpoint.put_checkpoint_l1_ref_blocking(commitment, observation.clone())?;

    // Update cached confirmed epoch monotonically.
    if state
        .confirmed_epoch
        .is_none_or(|current| commitment.epoch() > current.epoch())
    {
        state.confirmed_epoch = Some(commitment);
    }

    // Queue only non-finalized candidates and avoid duplicates.
    if state
        .finalized_epoch
        .is_none_or(|current| commitment.epoch() > current.epoch())
        && !state
            .observed_checkpoints
            .iter()
            .any(|(epoch, _)| *epoch == commitment)
    {
        state
            .observed_checkpoints
            .push_back((commitment, observation));
    }

    debug!(
        ?commitment,
        l1_height = asm_block.height(),
        ?checkpoint_txid,
        ?checkpoint_wtxid,
        "Recorded OL checkpoint L1 ref from v1 tip update"
    );
    Ok(())
}

fn newly_finalized_epoch(
    prev_final_epoch: Option<&EpochCommitment>,
    new_final_epoch: Option<&EpochCommitment>,
) -> Option<EpochCommitment> {
    // Return the newly declared finalized epoch only when finality advanced.
    match (prev_final_epoch, new_final_epoch) {
        (None, Some(new)) => Some(*new),
        (Some(old), Some(new)) if new.epoch() > old.epoch() => Some(*new),
        _ => None,
    }
}

/// Build a compatibility synthetic [`L1Checkpoint`] from a v1 checkpoint tip update.
// TODO(STR-2438): Remove this adapter once CSM consumes checkpoint-v1-native
// structures without legacy `L1Checkpoint` synthesis.
fn checkpoint_from_tip_update(
    checkpoint_tip_update: &CheckpointTipUpdate,
    asm_block: &L1BlockCommitment,
) -> L1Checkpoint {
    let tip = checkpoint_tip_update.tip();
    // TODO(STR-2952): populate real checkpoint wtxid here.
    // For now we mirror txid to preserve the required `CheckpointL1Ref` shape.
    let checkpoint_txid = *checkpoint_tip_update.checkpoint_txid();
    let checkpoint_wtxid = checkpoint_txid;
    let l1_reference = CheckpointL1Ref::new(*asm_block, checkpoint_txid, checkpoint_wtxid);

    // TODO(STR-2438): This v0-shaped `BatchInfo` synthesis is
    // semantically incorrect for checkpoint-v1 tip updates (start/end L1 and
    // L2 commitments are duplicated placeholders). Replace with v1-native data
    // flow and remove legacy `BatchInfo` construction.
    let batch_info = BatchInfo::new(
        tip.epoch,
        (*asm_block, *asm_block),
        (*tip.l2_commitment(), *tip.l2_commitment()),
    );

    L1Checkpoint::new(batch_info, l1_reference)
}


#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_asm_common::AsmLogEntry;
    use strata_asm_logs::{constants::DEPOSIT_LOG_TYPE_ID, CheckpointTipUpdate};
    use strata_checkpoint_types_ssz::CheckpointTip;
    use strata_csm_types::{ClientState, ClientUpdateOutput};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_params::{Params, RollupParams, SyncParams};
    use strata_primitives::{
        buf::Buf32,
        epoch::EpochCommitment,
        l1::BitcoinTxid,
        ol::{OLBlockCommitment, OLBlockId},
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
    }

    #[test]
    fn test_process_log_with_non_checkpoint_log_type() {
        let (mut state, _) = create_test_state();
        let asm_block = L1BlockCommitment::new(100, L1BlockId::default());

        let log = create_non_checkpoint_log_type();

        // Should succeed but do nothing
        let result = process_log(&mut state, &log, &asm_block);
        assert!(
            result.is_ok(),
            "process_log should ignore known non-checkpoint log types"
        );

        // State should not be updated
        assert_eq!(state.last_processed_epoch, None);
    }

    #[test]
    fn test_process_log_with_no_log_type() {
        let (mut state, _) = create_test_state();
        let asm_block = L1BlockCommitment::new(100, L1BlockId::default());

        let log = create_typeless_log();

        // Should succeed but do nothing
        let result = process_log(&mut state, &log, &asm_block);
        assert!(result.is_ok(), "process_log should handle typeless logs");

        // State should not be updated
        assert_eq!(state.last_processed_epoch, None);
    }

    #[test]
    fn test_process_sequential_checkpoint_tip_logs_happy_path() {
        let (mut state, _) = create_test_state();
        let mut arbgen = ArbitraryGenerator::new();

        for epoch in 1u32..=2u32 {
            let asm_block = L1BlockCommitment::new(200 + epoch, arbgen.generate());
            state.last_asm_block = Some(asm_block);

            let ol_tip = OLBlockCommitment::new(
                (epoch * 10) as u64,
                OLBlockId::from(Buf32::from([epoch as u8; 32])),
            );
            let tip = CheckpointTip::new(epoch, asm_block.height(), ol_tip);
            let tip_update = CheckpointTipUpdate::new(tip, Buf32::from([epoch as u8; 32]));
            let log = AsmLogEntry::from_log(&tip_update).expect("make tip log");

            let result = process_log(&mut state, &log, &asm_block);
            assert!(
                result.is_ok(),
                "process_log should succeed for tip epoch {}: {:?}",
                epoch,
                result
            );

            assert_eq!(
                state.last_processed_epoch,
                Some(epoch),
                "Last processed epoch should be updated to {}",
                epoch
            );
        }

        let declared_final_epoch = state
            .cur_state
            .as_ref()
            .get_declared_final_epoch()
            .expect("expected finalized epoch after two tip updates");
        assert_eq!(declared_final_epoch.epoch(), 1);
    }

    #[test]
    fn test_process_checkpoint_tip_log_writes_observation_without_payload() {
        let (mut state, storage) = create_test_state();

        let epoch = 9u32;
        let asm_block = L1BlockCommitment::new(250, L1BlockId::default());
        state.last_asm_block = Some(asm_block);

        let ol_tip = OLBlockCommitment::new(90, OLBlockId::from(Buf32::from([epoch as u8; 32])));
        let tip = CheckpointTip::new(epoch, asm_block.height(), ol_tip);
        let tip_update = CheckpointTipUpdate::new(tip, Buf32::from([epoch as u8; 32]));
        let log = AsmLogEntry::from_log(&tip_update).expect("make tip log");

        process_log(&mut state, &log, &asm_block).expect("tip log should process");

        let commitment = EpochCommitment::from_terminal(epoch, ol_tip);
        let observation = storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_blocking(commitment)
            .expect("query l1 ref");
        assert_eq!(
            observation.as_ref().map(|entry| entry.txid),
            Some(Buf32::from([epoch as u8; 32])),
            "l1 ref should persist checkpoint txid from v1 tip log"
        );
        assert_eq!(
            observation.as_ref().map(|entry| entry.wtxid),
            Some(Buf32::from([epoch as u8; 32])),
            "l1 ref should persist checkpoint wtxid from v1 tip log, which is same as txid(until STR-2952)"
        );
        assert_eq!(
            observation.as_ref().map(|entry| entry.l1_commitment),
            Some(asm_block),
            "l1 ref should be written from v1 tip log"
        );

        let payload = storage
            .ol_checkpoint()
            .get_checkpoint_payload_entry_blocking(commitment)
            .expect("query payload entry");
        assert!(
            payload.is_none(),
            "tip log path should not require payload entry to write observation"
        );
    }

}
