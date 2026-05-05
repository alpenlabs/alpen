//! Checkpoint log processing logic.

use std::sync::Arc;

use anyhow::Context;
use strata_asm_common::AsmLogEntry;
use strata_asm_logs::{CheckpointTipUpdate, constants::CHECKPOINT_TIP_UPDATE_LOG_TYPE};
use strata_checkpoint_types::BatchInfo;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
use strata_identifiers::Epoch;
use strata_primitives::prelude::*;
use tracing::*;

use crate::{
    checkpoint_extract::extract_matching_checkpoint, context::CsmWorkerContext,
    state::CsmWorkerState,
};

pub(crate) fn process_log<C: CsmWorkerContext>(
    state: &mut CsmWorkerState<C>,
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

/// Process a checkpoint tip update log from the checkpoint subprotocol.
fn process_checkpoint_tip_log<C: CsmWorkerContext>(
    state: &mut CsmWorkerState<C>,
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

    // Tip logs do not contain full batch transition details.
    // CSM only needs epoch progression for finalized-epoch signaling, so we
    // synthesize a minimal checkpoint view from the tip.
    // TODO(STR-2438): Remove this synthetic mapping once CSM persists/consumes
    // these fields directly without legacy L1Checkpoint shape coupling.
    let synthetic_checkpoint = checkpoint_from_tip_update(checkpoint_tip_update, asm_block);
    update_client_state_with_checkpoint(state, synthetic_checkpoint, epoch)?;
    mark_ol_checkpoint_l1_observed(state, checkpoint_tip_update, asm_block)?;

    state.last_processed_epoch = Some(epoch);
    Ok(())
}

/// Update and persist client state from a checkpoint.
fn update_client_state_with_checkpoint<C: CsmWorkerContext>(
    state: &mut CsmWorkerState<C>,
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
    state.ctx.put_client_state_update(
        &l1_block,
        ClientUpdateOutput::new(next_state.clone(), vec![]),
    )?;

    // Update our tracked state
    state.cur_state = Arc::new(next_state);

    // Update status channel
    state
        .ctx
        .publish_client_state(state.cur_state.as_ref().clone(), l1_block);

    Ok(())
}

fn mark_ol_checkpoint_l1_observed<C: CsmWorkerContext>(
    state: &mut CsmWorkerState<C>,
    checkpoint_tip_update: &CheckpointTipUpdate,
    asm_block: &L1BlockCommitment,
) -> anyhow::Result<()> {
    let tip = checkpoint_tip_update.tip();
    let _span = info_span!("mark_ol_checkpoint_l1_observed", epoch = tip.epoch).entered();
    let commitment = EpochCommitment::from_terminal(tip.epoch, *tip.l2_commitment());

    // ASM signed off on this block, so a fetch failure means the bitcoind RPC is
    // unreachable beyond what `get_l1_block`'s internal retry can absorb. Bubble
    // the error so the operator notices instead of losing the txid silently.
    let block = state
        .ctx
        .get_l1_block(asm_block.blkid())
        .with_context(|| format!("fetching L1 block {asm_block} for checkpoint observation"))?;

    let extracted = extract_matching_checkpoint(&block, state.ctx.magic_bytes(), tip)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no checkpoint envelope tx in L1 block {asm_block} matched the validated tip for epoch {}",
                tip.epoch
            )
        })?;

    let observation = CheckpointL1Ref::new(*asm_block, extracted.txid, extracted.wtxid);
    state
        .ctx
        .put_checkpoint_l1_observation(commitment, extracted.payload, observation.clone())?;

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
        txid = ?extracted.txid,
        wtxid = ?extracted.wtxid,
        "Recorded OL checkpoint L1 ref from tip update"
    );
    Ok(())
}

/// Build a compatibility synthetic [`L1Checkpoint`] from a checkpoint tip update.
// TODO(STR-2438): Remove this adapter once CSM consumes checkpoint tip update
// structures directly without legacy `L1Checkpoint` synthesis.
fn checkpoint_from_tip_update(
    checkpoint_tip_update: &CheckpointTipUpdate,
    asm_block: &L1BlockCommitment,
) -> L1Checkpoint {
    let tip = checkpoint_tip_update.tip();
    // Upstream tip logs do not include txid/wtxid.
    let checkpoint_txid = Buf32::zero();
    let checkpoint_wtxid = checkpoint_txid;
    let l1_reference = CheckpointL1Ref::new(*asm_block, checkpoint_txid, checkpoint_wtxid);

    // TODO(STR-2438): This `BatchInfo` synthesis is semantically incorrect
    // for checkpoint tip updates (start/end L1 and L2 commitments are
    // duplicated placeholders). Replace with native checkpoint data flow
    // and remove legacy `BatchInfo` construction.
    let batch_info = BatchInfo::new(
        tip.epoch,
        (*asm_block, *asm_block),
        (*tip.l2_commitment(), *tip.l2_commitment()),
    );

    L1Checkpoint::new(batch_info, l1_reference)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use bitcoin::{
        Block, BlockHash, CompactTarget, Transaction, TxMerkleNode,
        block::{Header, Version as BlockVersion},
        hashes::{Hash, sha256d},
    };
    use strata_asm_common::AsmLogEntry;
    use strata_asm_logs::{CheckpointTipUpdate, constants::DEPOSIT_LOG_TYPE_ID};
    use strata_asm_proto_checkpoint_txs::OL_STF_CHECKPOINT_TX_TAG;
    use strata_asm_proto_checkpoint_types::{
        CheckpointPayload, CheckpointSidecar, CheckpointTip, TerminalHeaderComplement,
    };
    use strata_asm_proto_txs_test_utils::create_reveal_transaction_stub;
    use strata_codec::encode_to_vec;
    use strata_codec_utils::CodecSsz;
    use strata_csm_types::{ClientState, ClientUpdateOutput};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_params::Params;
    use strata_primitives::{
        buf::Buf32,
        epoch::EpochCommitment,
        ol::{OLBlockCommitment, OLBlockId},
        prelude::*,
    };
    use strata_status::StatusChannel;
    use strata_storage::create_node_storage;
    use strata_test_utils::ArbitraryGenerator;

    use super::process_log;
    use crate::{state::CsmWorkerState, test_utils::StubCtx};

    fn create_test_params_arc() -> Arc<Params> {
        Arc::new(strata_test_utils_l2::gen_params())
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
        params: &Params,
        storage: Arc<strata_storage::NodeStorage>,
        status_channel: Arc<StatusChannel>,
    ) -> StubCtx {
        StubCtx::new(
            storage,
            status_channel,
            params.rollup.l1_reorg_safe_depth,
            params.rollup.magic_bytes,
        )
    }

    /// Helper to create a test CSM worker state with the default panicking stub ctx.
    fn create_test_state() -> (CsmWorkerState<StubCtx>, Arc<strata_storage::NodeStorage>) {
        let params = create_test_params_arc();
        let (storage, status_channel) = create_test_storage();
        let ctx = default_stub_ctx(&params, storage.clone(), status_channel);
        let state = CsmWorkerState::new(params, storage.clone(), ctx).unwrap();
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
        let state = CsmWorkerState::new(params, storage.clone(), ctx).unwrap();
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
            .expect("single-byte raw payload should produce typeless log")
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
        // Stage a matching envelope-bearing L1 block per epoch, keyed by the
        // synthetic L1BlockId we feed into `process_log`.
        let mut blocks_by_id = HashMap::new();
        let mut log_for_epoch = HashMap::new();
        for epoch in 1u32..=2u32 {
            let (log, _ol_tip, block, _tx) = tip_log_and_block_for_epoch(epoch);
            let blkid = L1BlockId::from(Buf32::from([epoch as u8; 32]));
            blocks_by_id.insert(blkid, block);
            log_for_epoch.insert(epoch, (blkid, log));
        }

        let (mut state, _) = create_test_state_with_ctx(|c| c.with_l1_blocks_by_id(blocks_by_id));

        for epoch in 1u32..=2u32 {
            let (blkid, log) = &log_for_epoch[&epoch];
            let asm_block = L1BlockCommitment::new(200 + epoch, *blkid);
            state.last_asm_block = Some(asm_block);

            process_log(&mut state, log, &asm_block).unwrap_or_else(|e| {
                panic!("process_log should succeed for tip epoch {epoch}: {e:?}")
            });

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

    /// Builds a tip log + matching block carrying a checkpoint envelope tx.
    fn tip_log_and_block_for_epoch(
        epoch: u32,
    ) -> (AsmLogEntry, OLBlockCommitment, Block, Transaction) {
        let ol_tip = OLBlockCommitment::new(
            epoch as u64 * 10,
            OLBlockId::from(Buf32::from([epoch as u8; 32])),
        );
        let payload_tip = CheckpointTip::new(epoch, 200, ol_tip);
        let sidecar = CheckpointSidecar::new(
            vec![],
            vec![],
            TerminalHeaderComplement::new(0, Buf32::zero().into(), Buf32::zero(), Buf32::zero()),
        )
        .expect("sidecar");
        let payload = CheckpointPayload::new(payload_tip, sidecar, vec![]).expect("payload");
        let bytes = encode_to_vec(&CodecSsz::new(payload)).expect("encode");
        let tx = create_reveal_transaction_stub(bytes, &OL_STF_CHECKPOINT_TX_TAG);

        let block = Block {
            header: Header {
                version: BlockVersion::TWO,
                prev_blockhash: BlockHash::from_raw_hash(sha256d::Hash::all_zeros()),
                merkle_root: TxMerkleNode::from_raw_hash(sha256d::Hash::all_zeros()),
                time: 0,
                bits: CompactTarget::from_consensus(0),
                nonce: 0,
            },
            txdata: vec![tx.clone()],
        };

        let tip_log = AsmLogEntry::from_log(&CheckpointTipUpdate::new(CheckpointTip::new(
            epoch, 200, ol_tip,
        )))
        .expect("tip log");

        (tip_log, ol_tip, block, tx)
    }

    #[test]
    fn writes_real_txid_and_wtxid_when_block_carries_matching_checkpoint() {
        let epoch = 9u32;
        let (log, ol_tip, block, tx) = tip_log_and_block_for_epoch(epoch);
        let asm_block = L1BlockCommitment::new(250, L1BlockId::default());

        let (mut state, storage) = create_test_state_with_ctx(|c| c.with_l1_block(block));
        state.last_asm_block = Some(asm_block);
        process_log(&mut state, &log, &asm_block).expect("tip log should process");

        let commitment = EpochCommitment::from_terminal(epoch, ol_tip);
        let observation = storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_blocking(commitment)
            .expect("query l1 ref")
            .expect("observation should be written");
        assert_eq!(
            observation.txid,
            Buf32::from(tx.compute_txid().to_byte_array())
        );
        assert_eq!(
            observation.wtxid,
            Buf32::from(tx.compute_wtxid().to_byte_array())
        );
        assert_eq!(observation.l1_commitment, asm_block);

        // The L1-observed payload is persisted alongside the L1 ref so future
        // checkpoint-sync consumers can reconstruct the checkpoint without
        // re-fetching from L1.
        let payload = storage
            .ol_checkpoint()
            .get_checkpoint_l1_observed_payload_blocking(commitment)
            .expect("query l1-observed payload")
            .expect("payload should be persisted");
        assert_eq!(payload.new_tip().epoch, epoch);
        assert_eq!(*payload.new_tip().l2_commitment(), ol_tip);
    }

    #[test]
    fn errors_when_l1_block_fetch_fails() {
        let epoch = 9u32;
        let (log, ol_tip, _block, _tx) = tip_log_and_block_for_epoch(epoch);
        let asm_block = L1BlockCommitment::new(250, L1BlockId::default());

        let (mut state, storage) = create_test_state_with_ctx(|c| c.with_l1_fetch_failure());
        state.last_asm_block = Some(asm_block);
        let err =
            process_log(&mut state, &log, &asm_block).expect_err("fetch failure should propagate");
        assert!(
            err.to_string().contains("fetching L1 block"),
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
    fn errors_when_no_matching_checkpoint_in_block() {
        let epoch = 9u32;
        let (log, ol_tip, _carrier_block, _tx) = tip_log_and_block_for_epoch(epoch);
        let empty_block = Block {
            header: Header {
                version: BlockVersion::TWO,
                prev_blockhash: BlockHash::from_raw_hash(sha256d::Hash::all_zeros()),
                merkle_root: TxMerkleNode::from_raw_hash(sha256d::Hash::all_zeros()),
                time: 0,
                bits: CompactTarget::from_consensus(0),
                nonce: 0,
            },
            txdata: vec![],
        };

        let asm_block = L1BlockCommitment::new(250, L1BlockId::default());
        let (mut state, storage) = create_test_state_with_ctx(|c| c.with_l1_block(empty_block));
        state.last_asm_block = Some(asm_block);
        let err = process_log(&mut state, &log, &asm_block)
            .expect_err("missing matching tx should propagate");
        assert!(
            err.to_string().contains("no checkpoint envelope tx"),
            "unexpected error: {err}"
        );

        let commitment = EpochCommitment::from_terminal(epoch, ol_tip);
        let observation = storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_blocking(commitment)
            .expect("query l1 ref");
        assert!(
            observation.is_none(),
            "no l1 ref should be written when no tx matches"
        );
    }

    #[test]
    fn observation_write_is_idempotent_on_repeat_log() {
        let epoch = 9u32;
        let (log, ol_tip, block, tx) = tip_log_and_block_for_epoch(epoch);
        let asm_block = L1BlockCommitment::new(250, L1BlockId::default());

        let (mut state, storage) = create_test_state_with_ctx(|c| c.with_l1_block(block));
        state.last_asm_block = Some(asm_block);

        process_log(&mut state, &log, &asm_block).expect("first tip log should process");
        process_log(&mut state, &log, &asm_block).expect("second tip log should process");

        let commitment = EpochCommitment::from_terminal(epoch, ol_tip);
        let observation = storage
            .ol_checkpoint()
            .get_checkpoint_l1_ref_blocking(commitment)
            .expect("query l1 ref")
            .expect("observation should be written");
        assert_eq!(
            observation.txid,
            Buf32::from(tx.compute_txid().to_byte_array())
        );

        let payload = storage
            .ol_checkpoint()
            .get_checkpoint_l1_observed_payload_blocking(commitment)
            .expect("query l1-observed payload")
            .expect("payload should be persisted");
        assert_eq!(payload.new_tip().epoch, epoch);
    }
}
