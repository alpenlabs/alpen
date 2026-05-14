//! Checkpoint log processing logic.

use std::sync::Arc;

use anyhow::Context;
use bitcoin::hashes::Hash;
use strata_asm_common::{AsmLogEntry, Subprotocol, VerifiedAuxData};
use strata_asm_logs::{CheckpointTipUpdate, constants::CHECKPOINT_TIP_UPDATE_LOG_TYPE};
use strata_asm_proto_checkpoint::{state::CheckpointState, subprotocol::CheckpointSubprotocol};
use strata_checkpoint_types::BatchInfo;
use strata_csm_types::{CheckpointL1Ref, ClientState, ClientUpdateOutput, L1Checkpoint};
use strata_identifiers::Epoch;
use strata_primitives::prelude::*;
use strata_state::asm_state::AsmState;
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

    mark_ol_checkpoint_l1_observed(state, checkpoint_tip_update, asm_block)?;
    // Tip logs do not contain full batch transition details.
    // CSM only needs epoch progression for finalized-epoch signaling, so we
    // synthesize a minimal checkpoint view from the tip.
    // TODO(STR-2438): Remove this synthetic mapping once CSM persists/consumes
    // these fields directly without legacy L1Checkpoint shape coupling.
    let synthetic_checkpoint = checkpoint_from_tip_update(checkpoint_tip_update, asm_block);
    apply_checkpoint_to_client_state(state, synthetic_checkpoint, epoch);

    state.last_processed_epoch = Some(epoch);
    Ok(())
}

/// Updates the in-memory client state.
///
/// This does NOT persist anything.
fn apply_checkpoint_to_client_state<C: CsmWorkerContext>(
    state: &mut CsmWorkerState<C>,
    new_checkpoint: L1Checkpoint,
    epoch: Epoch,
) {
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

    state.cur_state = Arc::new(ClientState::new(last_finalized, recent));
}

/// Persists the client state for `asm_block` and advances the in-memory `last_asm_block`.
///
/// Called after every log of the asm block processed without error.
pub(crate) fn commit_block<C: CsmWorkerContext>(
    state: &mut CsmWorkerState<C>,
    asm_block: L1BlockCommitment,
) -> anyhow::Result<()> {
    let next_state = state.cur_state.as_ref().clone();
    state.ctx.put_client_state_update(
        &asm_block,
        ClientUpdateOutput::new(next_state.clone(), vec![]),
    )?;
    state.last_asm_block = Some(asm_block);
    // Publish the client state to status channel.
    state.ctx.publish_client_state(next_state, asm_block);
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

    let block = state
        .ctx
        .get_l1_block(asm_block.blkid())
        .with_context(|| format!("fetching L1 block {asm_block} for checkpoint observation"))?;

    // Prepare for same checkpoint validation that ASM does.
    let parent_block = parent_commitment(asm_block, &block)?;
    let parent_asm_state = state.ctx.get_asm_state(&parent_block).with_context(|| {
        format!("fetching parent ASM state {parent_block} for checkpoint observation")
    })?;
    let checkpoint_state = decode_checkpoint_section(&parent_asm_state)?;
    let aux_data = state
        .ctx
        .get_aux_data(asm_block)
        .with_context(|| format!("fetching ASM aux data {asm_block} for checkpoint observation"))?;
    let verified_aux_data = VerifiedAuxData::try_new(
        &aux_data,
        &parent_asm_state.state().chain_view.history_accumulator,
    )
    .with_context(|| format!("verifying ASM aux data for {asm_block}"))?;

    let extracted = extract_matching_checkpoint(
        &block,
        state.ctx.magic_bytes(),
        tip,
        &checkpoint_state,
        asm_block.height(),
        &verified_aux_data,
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "no checkpoint envelope tx in L1 block {asm_block} validated for epoch {}",
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

/// Returns the parent L1 commitment derived from `block`'s header.
///
/// Fails for the genesis block where no parent exists; that case isn't
/// reachable in practice because epoch 0 produces no checkpoint tip update.
fn parent_commitment(
    asm_block: &L1BlockCommitment,
    block: &bitcoin::Block,
) -> anyhow::Result<L1BlockCommitment> {
    let height = asm_block.height();
    let parent_height = height
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("cannot derive parent for genesis L1 block {asm_block}"))?;
    let parent_blkid = L1BlockId::from(Buf32::from(block.header.prev_blockhash.to_byte_array()));
    Ok(L1BlockCommitment::new(parent_height, parent_blkid))
}

/// Extracts the checkpoint subprotocol's typed state from a parent `AsmState`.
fn decode_checkpoint_section(asm_state: &AsmState) -> anyhow::Result<CheckpointState> {
    asm_state
        .state()
        .find_section(CheckpointSubprotocol::ID)
        .ok_or_else(|| anyhow::anyhow!("checkpoint subprotocol section missing in ASM state"))?
        .try_to_state::<CheckpointSubprotocol>()
        .map_err(|e| anyhow::anyhow!("decode checkpoint subprotocol state: {e}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use strata_asm_common::AsmLogEntry;
    use strata_asm_logs::constants::DEPOSIT_LOG_TYPE_ID;
    use strata_csm_types::{ClientState, ClientUpdateOutput};
    use strata_db_store_sled::test_utils::get_test_sled_backend;
    use strata_params::Params;
    use strata_primitives::prelude::*;
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

    /// Synthesizes a CheckpointTipUpdate log for `epoch`. We use a placeholder
    /// `CheckpointTip` because tests using this helper short-circuit before
    /// reaching extraction, so the tip content is irrelevant.
    fn placeholder_tip_log(epoch: u32) -> (AsmLogEntry, OLBlockCommitment) {
        use strata_asm_logs::CheckpointTipUpdate;
        use strata_asm_proto_checkpoint_types::CheckpointTip;
        let ol_tip = OLBlockCommitment::new(
            epoch as u64 * 10,
            OLBlockId::from(Buf32::from([epoch as u8; 32])),
        );
        let log = AsmLogEntry::from_log(&CheckpointTipUpdate::new(CheckpointTip::new(
            epoch, 200, ol_tip,
        )))
        .expect("tip log");
        (log, ol_tip)
    }

    #[test]
    fn errors_when_l1_block_fetch_fails() {
        let epoch = 9u32;
        let (log, ol_tip) = placeholder_tip_log(epoch);
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
    fn commit_block_persists_state_and_advances_cursor() {
        let (mut state, storage) = create_test_state();
        let asm_block = L1BlockCommitment::new(300, L1BlockId::from(Buf32::from([7; 32])));

        super::commit_block(&mut state, asm_block).expect("commit should succeed");

        assert_eq!(state.last_asm_block, Some(asm_block));
        let (persisted_block, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state row should exist");
        assert_eq!(
            persisted_block, asm_block,
            "client-state row must be keyed on the committed block"
        );
    }

    #[test]
    fn failed_log_does_not_advance_persisted_cursor() {
        let epoch = 9u32;
        let (log, _ol_tip) = placeholder_tip_log(epoch);
        let asm_block = L1BlockCommitment::new(250, L1BlockId::default());

        let (mut state, storage) = create_test_state_with_ctx(|c| c.with_l1_fetch_failure());
        state.last_asm_block = Some(asm_block);

        // The genesis client-state row seeded by `create_test_storage`.
        let (before_block, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("seeded client state");

        // Simulate the service loop: a failing log means `commit_block` is
        // never called. Pin the failure to the L1-fetch path so the test
        // can't pass on an unrelated error.
        let err = process_log(&mut state, &log, &asm_block)
            .expect_err("L1 fetch failure should make the log fail");
        assert!(
            err.to_string().contains("fetching L1 block"),
            "unexpected error: {err}"
        );

        let (after_block, _) = storage
            .client_state()
            .fetch_most_recent_state()
            .expect("query client state")
            .expect("client state still present");
        assert_eq!(
            before_block, after_block,
            "persisted cursor must not move when a block's log failed"
        );
    }
}
