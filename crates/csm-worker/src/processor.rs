//! Checkpoint log processing logic.

use std::sync::Arc;

use anyhow::{Context, anyhow, bail};
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{CheckpointUpdate, constants::CHECKPOINT_UPDATE_LOG_TYPE};
use strata_checkpoint_types::{BatchInfo, BatchTransition, Checkpoint, CheckpointSidecar};
use strata_csm_types::{
    CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint, SyncAction,
};
use strata_identifiers::Epoch;
use strata_ledger_types::IStateAccessor;
use strata_ol_chain_types_new::OLL1ManifestContainer;
use strata_ol_da::{DecodedCheckpointDa, apply_da_payload, decode_checkpoint_da_blob};
use strata_ol_state_types::OLState;
use strata_ol_stf::{BasicExecContext, BlockInfo, ExecOutputBuffer, process_block_manifests};
use strata_primitives::prelude::*;
use tracing::*;

use crate::{state::CsmWorkerState, sync_actions::apply_action};

/// Dispatches an ASM log to checkpoint processing when the log type is supported.
///
/// Unsupported or typeless logs are ignored without mutating CSM state.
pub(crate) fn process_log(
    state: &mut CsmWorkerState,
    log: &AsmLogEntry,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    match log.ty() {
        Some(CHECKPOINT_UPDATE_LOG_TYPE) => {
            let ckpt_upd = log
                .try_into_log()
                .map_err(|e| anyhow!("Failed to deserialize CheckpointUpdate: {e}"))?;

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

    if state.use_legacy_l2_pre_state {
        // Keep legacy sync behavior unchanged for old functional tests and old sync consumers:
        // process checkpoint metadata/finalization without consuming checkpoint DA.
        debug!(
            %epoch,
            "legacy mode enabled, skipping checkpoint DA application"
        );
    } else {
        // Apply and verify DA first so the worker fails closed: if fetching/decoding/state-root
        // checks fail, this handler does not persist checkpoint-inclusion metadata or advance
        // client state.
        apply_checkpoint_da_update(state, checkpoint_update)?;
    }

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

/// Fetches, decodes, validates, applies, and persists OL DA for one checkpoint update.
///
/// This routine reconstructs the checkpoint preseal transition from L1 data and validates it
/// against ASM-provided commitments before any checkpoint inclusion metadata is written:
///
/// 1. Fetches the checkpoint transaction from the configured Bitcoin client tx provider.
/// 2. Decodes and validates the checkpoint DA payload from the sidecar.
/// 3. Resolves checkpoint pre-state and verifies the pre-state root.
/// 4. Applies DA payload, then applies L1-derived epoch sealing updates.
/// 5. Verifies the post-state root and persists the resulting OL state snapshot.
fn apply_checkpoint_da_update(
    state: &mut CsmWorkerState,
    checkpoint_update: &CheckpointUpdate,
) -> anyhow::Result<()> {
    let raw_tx = state
        .get_bitcoin_tx(checkpoint_update.checkpoint_txid())
        .with_context(|| {
            format!(
                "failed to fetch checkpoint transaction {:?}",
                checkpoint_update.checkpoint_txid()
            )
        })?;

    let decoded = decode_checkpoint_da_blob(&raw_tx, state._params.rollup().magic_bytes)
        .context("failed to decode checkpoint DA payload from L1 transaction")?;

    validate_checkpoint_payload_tip(checkpoint_update, &decoded)?;

    let (pre_state_commitment, mut pre_state) =
        resolve_pre_state_for_checkpoint(state, checkpoint_update)
            .context("failed to resolve checkpoint pre-state")?;
    let pre_root = pre_state
        .compute_state_root()
        .context("failed to compute pre-state root")?;
    if pre_root != checkpoint_update.chainstate_transition().pre_state_root {
        bail!(
            "pre-state root mismatch for epoch {}: expected {}, got {} (pre-state commitment: {})",
            checkpoint_update.batch_info().epoch(),
            checkpoint_update.chainstate_transition().pre_state_root,
            pre_root,
            pre_state_commitment
        );
    }

    apply_da_payload(&mut pre_state, decoded.da_payload)
        .context("failed to apply OL DA payload")?;
    apply_epoch_sealing_updates(
        &state.storage,
        checkpoint_update.batch_info(),
        &mut pre_state,
    )
    .context("failed to apply epoch sealing updates from ASM manifests")?;

    let post_root = pre_state
        .compute_state_root()
        .context("failed to compute post-state root")?;
    if post_root != checkpoint_update.chainstate_transition().post_state_root {
        bail!(
            "post-state root mismatch for epoch {}: expected {}, got {}",
            checkpoint_update.batch_info().epoch(),
            checkpoint_update.chainstate_transition().post_state_root,
            post_root
        );
    }

    let terminal_commitment = *decoded.signed_checkpoint.inner().new_tip().l2_commitment();
    state
        .storage
        .ol_state()
        .put_toplevel_ol_state_blocking(terminal_commitment, pre_state)
        .with_context(|| {
            format!(
                "failed to persist OL state for checkpoint terminal commitment {}",
                terminal_commitment
            )
        })?;

    Ok(())
}

/// Validates that the decoded checkpoint payload tip matches ASM checkpoint update metadata.
fn validate_checkpoint_payload_tip(
    checkpoint_update: &CheckpointUpdate,
    decoded: &DecodedCheckpointDa,
) -> anyhow::Result<()> {
    let payload_tip = decoded.signed_checkpoint.inner().new_tip();
    let expected_batch = checkpoint_update.batch_info();
    let expected_l1_height = expected_batch.final_l1_block().height_u32();
    let expected_l2_commitment = expected_batch.final_l2_block();

    if payload_tip.epoch != expected_batch.epoch() {
        bail!(
            "checkpoint epoch mismatch: payload={} asm_log={}",
            payload_tip.epoch,
            expected_batch.epoch()
        );
    }

    if payload_tip.l1_height() != expected_l1_height {
        bail!(
            "checkpoint L1 height mismatch: payload={} asm_log={}",
            payload_tip.l1_height(),
            expected_l1_height
        );
    }

    if payload_tip.l2_commitment() != expected_l2_commitment {
        bail!(
            "checkpoint L2 commitment mismatch: payload={} asm_log={}",
            payload_tip.l2_commitment(),
            expected_l2_commitment
        );
    }

    Ok(())
}

/// Resolves the checkpoint pre-state snapshot used for DA application.
///
/// The pre-state is derived from the parent of `batch_info.l2_range.0`.
///
/// In default mode this resolves strictly from the `ol_block` store. In legacy mode this first
/// tries legacy `l2` resolution and then falls back to legacy snapshot lookup behavior for
/// compatibility with old sync paths.
fn resolve_pre_state_for_checkpoint(
    state: &CsmWorkerState,
    checkpoint_update: &CheckpointUpdate,
) -> anyhow::Result<(OLBlockCommitment, OLState)> {
    if state.use_legacy_l2_pre_state {
        return resolve_pre_state_for_checkpoint_legacy(state, checkpoint_update);
    }

    let l2_start = checkpoint_update.batch_info().l2_range.0;
    let pre_state_commitment = resolve_pre_state_commitment_from_ol_block(state, l2_start)?;
    let pre_state = state
        .storage
        .ol_state()
        .get_toplevel_ol_state_blocking(pre_state_commitment)?
        .ok_or_else(|| {
            anyhow!(
                "missing OL state snapshot for checkpoint pre-state commitment {pre_state_commitment}"
            )
        })?;
    Ok((pre_state_commitment, pre_state.as_ref().clone()))
}

/// Resolves checkpoint pre-state in legacy mode with compatibility fallbacks.
///
/// This tries strict `l2`-derived parent resolution first. If the derived commitment cannot be
/// mapped to an OL snapshot, it falls back to previous-checkpoint terminal state, then latest OL
/// snapshot.
// TODO: remove this once we delete the "old" code and functional tests
fn resolve_pre_state_for_checkpoint_legacy(
    state: &CsmWorkerState,
    checkpoint_update: &CheckpointUpdate,
) -> anyhow::Result<(OLBlockCommitment, OLState)> {
    let l2_start = checkpoint_update.batch_info().l2_range.0;

    match resolve_pre_state_commitment_from_legacy_l2(state, l2_start) {
        Ok(pre_state_commitment) => {
            if let Some(pre_state) = state
                .storage
                .ol_state()
                .get_toplevel_ol_state_blocking(pre_state_commitment)?
            {
                return Ok((pre_state_commitment, pre_state.as_ref().clone()));
            }
            warn!(
                %pre_state_commitment,
                %l2_start,
                "legacy l2-derived pre-state commitment missing in OL snapshots; falling back"
            );
        }
        Err(error) => {
            warn!(
                %l2_start,
                ?error,
                "failed to resolve legacy l2-derived pre-state; falling back"
            );
        }
    }

    let epoch = checkpoint_update.batch_info().epoch();
    if epoch > 0 {
        let prev_epoch = (epoch - 1) as u64;
        if let Some(prev_checkpoint) = state
            .storage
            .checkpoint()
            .get_checkpoint_blocking(prev_epoch)?
        {
            let prev_terminal = *prev_checkpoint.checkpoint.batch_info().final_l2_block();
            if let Some(prev_state) = state
                .storage
                .ol_state()
                .get_toplevel_ol_state_blocking(prev_terminal)?
            {
                return Ok((prev_terminal, prev_state.as_ref().clone()));
            }
            warn!(
                %prev_terminal,
                prev_epoch,
                "legacy previous-checkpoint pre-state missing; falling back to latest snapshot"
            );
        }
    }

    let (commitment, state_snapshot) = state
        .storage
        .ol_state()
        .get_latest_toplevel_ol_state_blocking()?
        .ok_or_else(|| anyhow!("missing OL state snapshot for checkpoint DA pre-state"))?;

    Ok((commitment, state_snapshot.as_ref().clone()))
}

/// Resolves pre-state commitment from the legacy `l2` block store.
// TODO: remove this once we delete the "old" code and functional tests
fn resolve_pre_state_commitment_from_legacy_l2(
    state: &CsmWorkerState,
    l2_start: OLBlockCommitment,
) -> anyhow::Result<OLBlockCommitment> {
    let bundle = state
        .storage
        .l2()
        .get_block_data_blocking(l2_start.blkid())?
        .ok_or_else(|| {
            anyhow!(
                "missing checkpoint L2 start block in legacy l2 store for commitment {}",
                l2_start
            )
        })?;
    let header = bundle.block().header().header();
    if header.slot() != l2_start.slot() {
        bail!(
            "checkpoint L2 start slot mismatch (legacy l2 store): batch_info={} block_header={}",
            l2_start.slot(),
            header.slot()
        );
    }

    let pre_state_slot = resolve_pre_state_slot(l2_start)?;
    Ok(OLBlockCommitment::new(pre_state_slot, header.prev_block()))
}

/// Resolves pre-state commitment from the new `ol_block` block store.
fn resolve_pre_state_commitment_from_ol_block(
    state: &CsmWorkerState,
    l2_start: OLBlockCommitment,
) -> anyhow::Result<OLBlockCommitment> {
    let block = state
        .storage
        .ol_block()
        .get_block_data_blocking(*l2_start.blkid())?
        .ok_or_else(|| {
            anyhow!(
                "missing checkpoint L2 start block in ol_block store for commitment {}",
                l2_start
            )
        })?;
    let header = block.header();
    if header.slot() != l2_start.slot() {
        bail!(
            "checkpoint L2 start slot mismatch (ol_block store): batch_info={} block_header={}",
            l2_start.slot(),
            header.slot()
        );
    }

    let pre_state_slot = resolve_pre_state_slot(l2_start)?;
    Ok(OLBlockCommitment::new(
        pre_state_slot,
        *header.parent_blkid(),
    ))
}

/// Resolves the pre-state slot from the checkpoint range start slot.
fn resolve_pre_state_slot(l2_start: OLBlockCommitment) -> anyhow::Result<u64> {
    l2_start
        .slot()
        .checked_sub(1)
        .ok_or_else(|| anyhow!("invalid checkpoint L2 start slot 0 for pre-state resolution"))
}

/// Applies deterministic L1-derived epoch sealing updates from ASM manifests to OL state.
fn apply_epoch_sealing_updates(
    storage: &Arc<strata_storage::NodeStorage>,
    batch_info: &BatchInfo,
    ol_state: &mut OLState,
) -> anyhow::Result<()> {
    let start_height = batch_info.l1_range.0.height_u64();
    let end_height = batch_info.l1_range.1.height_u64();
    if end_height < start_height {
        bail!(
            "invalid L1 range in checkpoint batch info: start={} end={}",
            start_height,
            end_height
        );
    }

    let mut manifests = Vec::new();
    for height in start_height..=end_height {
        let manifest = storage
            .l1()
            .get_block_manifest_at_height(height)?
            .ok_or_else(|| anyhow!("missing ASM manifest at L1 height {}", height))?;
        manifests.push(manifest);
    }

    let manifest_container = OLL1ManifestContainer::new(manifests)
        .context("failed to construct OL manifest container")?;

    let terminal_block = batch_info.final_l2_block();
    let block_info = BlockInfo::new(0, terminal_block.slot(), batch_info.epoch());
    let output_buffer = ExecOutputBuffer::new_empty();
    let basic_context = BasicExecContext::new(block_info, &output_buffer);
    process_block_manifests(ol_state, &manifest_container, &basic_context)
        .context("manifest processing failed")?;

    Ok(())
}

/// Update client state with a new checkpoint.
fn update_client_state_with_checkpoint(
    state: &mut CsmWorkerState,
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
    use ssz::Encode;
    use strata_asm_common::AsmLogEntry;
    use strata_asm_logs::{CheckpointUpdate, constants::CHECKPOINT_UPDATE_LOG_TYPE};
    use strata_asm_manifest_types::AsmManifest;
    use strata_asm_proto_checkpoint_txs::{
        CHECKPOINT_V0_SUBPROTOCOL_ID, OL_STF_CHECKPOINT_TX_TYPE,
    };
    use strata_checkpoint_types::{BatchInfo, ChainstateRootTransition};
    use strata_csm_types::{ClientState, ClientUpdateOutput};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_identifiers::WtxidsRoot;
    use strata_ledger_types::IStateAccessor;
    use strata_ol_chain_types_new::{
        BlockFlags, OLBlock, OLBlockBody, OLBlockHeader, OLTxSegment, SignedOLBlockHeader,
    };
    use strata_ol_da::{
        OLDaPayloadV1, StateDiff, apply_da_payload,
        test_utils::{make_checkpoint_tx, make_signed_checkpoint_payload},
    };
    use strata_ol_state_types::OLState;
    use strata_params::{Params, RollupParams, SyncParams};
    use strata_primitives::{
        buf::{Buf32, Buf64},
        epoch::EpochCommitment,
        l1::{BitcoinTxid, RawBitcoinTx},
        l2::{L2BlockCommitment, L2BlockId},
        prelude::*,
    };
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
    use strata_test_utils::ArbitraryGenerator;

    use super::{apply_epoch_sealing_updates, process_log};
    use crate::state::CsmWorkerState;

    /// Creates a test state for CSM worker.
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

        let params = Arc::new(Params {
            rollup: rollup_params,
            run: SyncParams {
                l1_follow_distance: 10,
                client_checkpoint_interval: 100,
                l2_blocks_fetch_limit: 1000,
            },
        });

        let db = get_test_sled_backend();
        let pool = threadpool::ThreadPool::new(4);
        let storage = Arc::new(create_node_storage(db, pool).expect("Failed to create storage"));

        let initial_state = ClientState::new(None, None);
        let initial_block = L1BlockCommitment::new(Height::ZERO, L1BlockId::default());
        storage
            .client_state()
            .put_update_blocking(
                &initial_block,
                ClientUpdateOutput::new(initial_state.clone(), vec![]),
            )
            .expect("Failed to initialize client state");

        let genesis_commitment = OLBlockCommitment::new(0, L2BlockId::from(Buf32::from([9u8; 32])));
        storage
            .ol_state()
            .put_toplevel_ol_state_blocking(genesis_commitment, OLState::new_genesis())
            .expect("Failed to initialize OL genesis state");

        let mut arbgen = ArbitraryGenerator::new();
        let status_channel = StatusChannel::new(
            arbgen.generate(),
            arbgen.generate(),
            arbgen.generate(),
            None,
            None,
        );

        let state = CsmWorkerState::new_for_tests(params, storage.clone(), status_channel.into())
            .expect("create csm state");

        (state, storage)
    }

    /// Creates a log with an unknown type.
    fn create_unknown_log_type() -> AsmLogEntry {
        AsmLogEntry::from_msg(999, vec![1, 2, 3, 4]).expect("Failed to create log")
    }

    /// Creates a log with no type.
    fn create_typeless_log() -> AsmLogEntry {
        AsmLogEntry::from_raw(vec![5, 6, 7, 8])
    }

    /// Tests that `process_log` handles unknown log types.
    #[test]
    fn test_process_log_with_unknown_log_type() {
        let (mut state, _) = create_test_state();
        let asm_block =
            L1BlockCommitment::new(Height::from_consensus(100).unwrap(), L1BlockId::default());

        let log = create_unknown_log_type();
        let result = process_log(&mut state, &log, &asm_block);
        assert!(result.is_ok(), "process_log should handle unknown types");
        assert_eq!(state.last_processed_epoch, None);
    }

    /// Tests that `process_log` handles typeless logs.
    #[test]
    fn test_process_log_with_no_log_type() {
        let (mut state, _) = create_test_state();
        let asm_block =
            L1BlockCommitment::new(Height::from_consensus(100).unwrap(), L1BlockId::default());

        let log = create_typeless_log();
        let result = process_log(&mut state, &log, &asm_block);
        assert!(result.is_ok(), "process_log should handle typeless logs");
        assert_eq!(state.last_processed_epoch, None);
    }

    /// Tests that `process_log` handles invalid checkpoint data.
    #[test]
    fn test_process_log_with_invalid_checkpoint_data() {
        let (mut state, _) = create_test_state();
        let asm_block =
            L1BlockCommitment::new(Height::from_consensus(100).unwrap(), L1BlockId::default());
        state.last_asm_block = Some(asm_block);

        let invalid_log = AsmLogEntry::from_msg(CHECKPOINT_UPDATE_LOG_TYPE, vec![1, 2, 3])
            .expect("Failed to create log");

        let result = process_log(&mut state, &invalid_log, &asm_block);
        assert!(
            result.is_err(),
            "process_log should fail with invalid checkpoint data"
        );
        assert!(
            result
                .expect_err("invalid checkpoint data should error")
                .to_string()
                .contains("Failed to deserialize CheckpointUpdate"),
            "Error should mention deserialization failure"
        );
    }

    /// Tests that `process_log` handles sequential checkpoint logs.
    #[test]
    fn test_process_sequential_checkpoint_logs_happy_path() {
        let secret_key = Buf32::from([1u8; 32]);
        let (mut state, storage) = create_test_state();
        let mut arbgen = ArbitraryGenerator::new();

        let mut prev_terminal = storage
            .ol_state()
            .get_latest_toplevel_ol_state_blocking()
            .expect("read latest ol state")
            .expect("genesis ol state must exist")
            .0;

        for epoch in 1u32..=3u32 {
            let asm_block = L1BlockCommitment::new(
                Height::from_consensus(100 + epoch).unwrap(),
                arbgen.generate(),
            );
            state.last_asm_block = Some(asm_block);

            let l2_start = seed_ol_block(
                &storage,
                ((epoch - 1) * 10 + 1) as u64,
                epoch,
                *prev_terminal.blkid(),
            );
            let l2_end = L2BlockCommitment::new(
                (epoch * 10) as u64,
                L2BlockId::from(Buf32::from([(epoch + 1) as u8; 32])),
            );

            let l1_height = 100 + epoch as u64;
            let l1_blkid = L1BlockId::from(Buf32::from([epoch as u8; 32]));
            seed_manifest(&storage, l1_height, l1_blkid);
            let l1_comm =
                L1BlockCommitment::new(Height::from_consensus(l1_height as u32).unwrap(), l1_blkid);

            let batch_info = BatchInfo::new(epoch, (l1_comm, l1_comm), (l2_start, l2_end));

            let mut expected_state = storage
                .ol_state()
                .get_toplevel_ol_state_blocking(prev_terminal)
                .expect("read pre-state")
                .expect("pre-state must exist")
                .as_ref()
                .clone();
            let pre_root = expected_state
                .compute_state_root()
                .expect("compute pre-state root");
            apply_da_payload(
                &mut expected_state,
                OLDaPayloadV1::new(StateDiff::default()),
            )
            .expect("apply da payload");
            apply_epoch_sealing_updates(&storage, &batch_info, &mut expected_state)
                .expect("apply epoch sealing");
            let post_root = expected_state
                .compute_state_root()
                .expect("compute post-state root");

            let chainstate_transition = ChainstateRootTransition {
                pre_state_root: pre_root,
                post_state_root: post_root,
            };

            let checkpoint_txid: BitcoinTxid = arbgen.generate();
            let signed_checkpoint = make_signed_checkpoint_payload(
                epoch,
                l1_height as u32,
                l2_end,
                strata_codec::encode_to_vec(&OLDaPayloadV1::new(StateDiff::default()))
                    .expect("encode da payload"),
                secret_key,
            );
            state.insert_checkpoint_tx_fixture(
                checkpoint_txid.clone(),
                RawBitcoinTx::from(make_checkpoint_tx(
                    &signed_checkpoint.as_ssz_bytes(),
                    CHECKPOINT_V0_SUBPROTOCOL_ID,
                    OL_STF_CHECKPOINT_TX_TYPE,
                    secret_key,
                )),
            );

            let checkpoint_update = CheckpointUpdate::new(
                EpochCommitment::from_terminal(epoch, l2_end),
                batch_info,
                chainstate_transition,
                checkpoint_txid,
            );

            let log = AsmLogEntry::from_log(&checkpoint_update).expect("make log");
            let result = process_log(&mut state, &log, &asm_block);
            assert!(
                result.is_ok(),
                "process_log should succeed for epoch {}: {:?}",
                epoch,
                result
            );

            assert_eq!(
                state.last_processed_epoch,
                Some(epoch),
                "Last processed epoch should be updated to {}",
                epoch
            );

            let stored_checkpoint = storage
                .checkpoint()
                .get_checkpoint_blocking(epoch as u64)
                .expect("Failed to query checkpoint database");
            assert!(
                stored_checkpoint.is_some(),
                "Checkpoint for epoch {} should be stored in database",
                epoch
            );

            let persisted_state = storage
                .ol_state()
                .get_toplevel_ol_state_blocking(l2_end)
                .expect("read persisted state")
                .expect("persisted state should exist");
            assert_eq!(
                persisted_state
                    .compute_state_root()
                    .expect("compute persisted state root"),
                post_root,
                "Persisted state root should match checkpoint post root for epoch {}",
                epoch
            );

            prev_terminal = l2_end;
        }
    }

    /// Seeds a manifest into the storage.
    fn seed_manifest(storage: &Arc<strata_storage::NodeStorage>, height: u64, blkid: L1BlockId) {
        let manifest = AsmManifest::new(height, blkid, WtxidsRoot::from(Buf32::zero()), vec![]);
        storage
            .l1()
            .put_block_data(manifest)
            .expect("store manifest");
        storage
            .l1()
            .extend_canonical_chain(&blkid, height)
            .expect("extend canonical chain");
    }

    /// Seeds the first OL block in a checkpoint range and returns its commitment.
    fn seed_ol_block(
        storage: &Arc<strata_storage::NodeStorage>,
        slot: u64,
        epoch: u32,
        parent_blkid: L2BlockId,
    ) -> L2BlockCommitment {
        let header = OLBlockHeader::new(
            0,
            BlockFlags::zero(),
            slot,
            epoch,
            parent_blkid,
            Buf32::zero(),
            Buf32::zero(),
            Buf32::zero(),
        );
        let body = OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("construct tx segment"));
        let block = OLBlock::new(SignedOLBlockHeader::new(header, Buf64::zero()), body);
        let commitment = block.header().compute_block_commitment();

        storage
            .ol_block()
            .put_block_data_blocking(block)
            .expect("store OL block");

        commitment
    }
}
