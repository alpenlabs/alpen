//! Service state for the chain worker.
//!
//! This module contains the state management for the chain worker service.
//! The state is internally organized into:
//! - [`ChainWorkerDeps`]: Static dependencies (context, params, runtime handles)
//! - [`ChainWorkerMutableState`]: Actual mutable state (tip, epoch info, etc.)
//!
//! This separation makes it clear which parts are actual "state" vs dependencies,
//! even though both must live in [`ChainWorkerServiceState`] due to the current
//! service framework design.

use std::collections::HashMap;

use strata_acct_types::AccountSerial;
use strata_asm_common::AsmManifest;
use strata_asm_proto_checkpoint_types::{CheckpointSidecar, CheckpointTip};
use strata_bridge_params::BridgeParams;
use strata_checkpoint_types::EpochSummary;
use strata_db_types::errors::DbError;
use strata_identifiers::{AccountId, Buf32, Epoch, OLBlockCommitment};
use strata_ledger_types::{
    IAccountState, ISnarkAccountState, IStateAccessor, StateError, StateResult,
};
use strata_msg_fmt::{Msg, MsgRef};
use strata_ol_chain_types::{
    BlockFlags, MAX_SEALING_MANIFEST_COUNT, OLBlock, OLBlockHeader, OLLog, OLLogType,
    SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID, SnarkAccountUpdateLogData,
};
use strata_ol_da::{OLDaSchemeV1, decode_ol_da_payload_bytes};
use strata_ol_state_support_types::{
    IndexerState, IndexerWrites, MemoryStateBaseLayer, SnarkAcctStateUpdate, WriteTrackingState,
};
use strata_ol_state_types::{IStateBatchApplicable, OLAccountState, OLState, WriteBatch};
use strata_ol_stf::{BlockInfo, EpochInfo, apply_da_epoch, verify_block};
use strata_primitives::{epoch::EpochCommitment, l1::L1BlockCommitment};
use strata_service::ServiceState;
use strata_snark_acct_types::Seqno;
use tracing::*;

use crate::{
    ChainWorkerContextImpl,
    errors::{WorkerError, WorkerResult},
    output::OLBlockExecutionOutput,
    traits::ChainWorkerContext,
};

#[derive(Clone, Copy, Default)]
struct SnarkAccountCursor {
    seqno: Seqno,
    next_inbox_msg_idx: u64,
}

type SnarkAccountCursors = HashMap<AccountSerial, SnarkAccountCursor>;

#[derive(Clone)]
struct ReconstructedSnarkData {
    /// Snark acct updates as they appear in OL logs.
    updates: Vec<SnarkAcctStateUpdate>,
    /// New cursors(next inbox id and seq no) for each account in logs.
    acct_cursors: SnarkAccountCursors,
}

/// Mutable state for the chain worker.
///
/// This contains the actual "state" - data that changes during the worker's
/// operation and represents the current processing position.
#[derive(Debug)]
struct ChainWorkerMutableState {
    /// Current tip commitment.
    cur_tip: OLBlockCommitment,

    /// Last finalized epoch, if any.
    last_finalized_epoch: Option<EpochCommitment>,

    /// Whether the worker has been initialized.
    initialized: bool,
}

impl Default for ChainWorkerMutableState {
    fn default() -> Self {
        Self {
            cur_tip: OLBlockCommitment::null(),
            last_finalized_epoch: None,
            initialized: false,
        }
    }
}

/// Service state for the chain worker.
///
/// This combines static dependencies with mutable state. The separation is
/// internal to make the code clearer about what is actual "state" vs what
/// are just dependencies needed for operations.
#[expect(
    missing_debug_implementations,
    reason = "Some inner types don't have Debug impl"
)]
pub struct ChainWorkerServiceState {
    /// Static dependencies.
    ctx: ChainWorkerContextImpl,

    /// Mutable state.
    state: ChainWorkerMutableState,
}

impl ChainWorkerServiceState {
    /// Creates a new chain worker service state.
    pub fn new(ctx: ChainWorkerContextImpl) -> Self {
        Self {
            ctx,
            state: ChainWorkerMutableState::default(),
        }
    }

    /// Returns whether the worker has been initialized.
    pub(crate) fn is_initialized(&self) -> bool {
        self.state.initialized
    }

    fn check_initialized(&self) -> WorkerResult<()> {
        if !self.is_initialized() {
            Err(WorkerError::NotInitialized)
        } else {
            Ok(())
        }
    }

    /// Returns the current tip commitment.
    pub(crate) fn cur_tip(&self) -> OLBlockCommitment {
        self.state.cur_tip
    }

    /// Returns the last finalized epoch, if any.
    pub(crate) fn last_finalized_epoch(&self) -> Option<EpochCommitment> {
        self.state.last_finalized_epoch
    }

    /// Waits for genesis and resolves the initial tip commitment.
    ///
    /// This first checks the database for an existing chain tip (highest executed block).
    /// If found, it resumes from there. Otherwise, it waits for genesis and starts fresh.
    pub(crate) fn wait_for_genesis_and_resolve_tip(&self) -> WorkerResult<OLBlockCommitment> {
        // First, check if we have an existing chain tip in the database.
        // This allows us to resume from where we left off after a restart,
        // including unfinalized blocks.
        if let Some(db_tip) = self.ctx.fetch_chain_tip()? {
            info!(slot = db_tip.slot(), %db_tip, "resuming from database chain tip");
            return Ok(db_tip);
        }

        // No existing chain - wait for genesis
        info!("waiting until genesis");

        let _init_state = self
            .ctx
            .handle()
            .block_on(self.ctx.status_channel().wait_until_genesis())
            .map_err(|_| WorkerError::ShutdownBeforeGenesis)?;

        // Start from genesis block
        let genesis_block_ids = self.ctx.fetch_blocks_at_slot(0)?;
        let genesis_blkid = *genesis_block_ids
            .first()
            .ok_or(WorkerError::MissingGenesisBlock)?;

        Ok(OLBlockCommitment::new(0, genesis_blkid))
    }

    /// Initializes the worker with the given tip commitment.
    pub(crate) fn initialize_with_tip(&mut self, cur_tip: OLBlockCommitment) -> anyhow::Result<()> {
        let blkid = *cur_tip.blkid();
        info!(%blkid, "initializing chain worker");

        // Seed the DB-side L1 block refs MMR mirror with the same sentinel
        // prefix the in-state MMR was given at OL genesis. Idempotent across
        // restarts.
        self.ctx.prefill_l1_block_refs_mmr()?;

        self.state.cur_tip = cur_tip;
        self.state.initialized = true;

        Ok(())
    }

    /// Tries to execute a block using the new OL STF.
    pub(crate) fn try_exec_block(
        &mut self,
        block_commitment: &OLBlockCommitment,
    ) -> WorkerResult<()> {
        self.check_initialized()?;
        exec_block(&self.ctx, self.ctx.bridge_params(), block_commitment)
    }

    /// Updates the current tip as managed by the worker.
    pub(crate) fn update_cur_tip(&mut self, tip: OLBlockCommitment) -> WorkerResult<()> {
        self.state.cur_tip = tip;
        Ok(())
    }

    /// Marks an epoch as finalized(buried).
    ///
    /// By the time this runs the epoch's terminal state has already been merged
    /// and stored — by [`handle_terminal_block_exec_post_ops`] for full
    /// sync, or by `apply_checkpoint` for checkpoint sync. This verifies that
    /// state is present, then records the epoch as finalized.
    pub(crate) fn finalize_epoch(&mut self, epoch: EpochCommitment) -> WorkerResult<()> {
        let terminal = epoch.to_block_commitment();
        if self.ctx.fetch_ol_state(terminal)?.is_none() {
            return Err(WorkerError::MissingPreState(terminal));
        }
        self.state.last_finalized_epoch = Some(epoch);
        Ok(())
    }

    /// Reconstructs an epoch's OL state from its checkpoint and persists it. Used by checkpoint
    /// sync.
    #[instrument(
        level = "debug",
        skip_all,
        fields(epoch = epoch.epoch(), slot = epoch.last_slot(), blkid = %epoch.last_blkid()),
        err
    )]
    pub(crate) fn apply_checkpoint(&mut self, epoch: EpochCommitment) -> WorkerResult<()> {
        let artifacts = apply_checkpoint_epoch(&self.ctx, epoch)?;

        self.ctx
            .store_toplevel_state(artifacts.terminal, artifacts.new_state)?;
        self.ctx.apply_epoch_indexing(&epoch, &artifacts.output)?;
        // Store the summary last to indicate the epoch has been processed and applied. If this
        // fails, this will be applied again, which is idempotent in db operations.
        self.ctx.store_summary(artifacts.summary)?;

        Ok(())
    }
}

/// Executes a block via the OL STF and persists its outputs.
///
/// For terminal blocks the execution output is persisted *before* the epoch
/// post-ops run: [`handle_terminal_block_exec_post_ops`] stamps the epoch
/// commitment onto the indexing row that persisting a block of the epoch
/// creates. In a single-block epoch (e.g. one sealed immediately by the
/// checkpoint size policy) this block's persist is the only thing that
/// creates that row, so stamping first fails with a missing-row error.
pub(crate) fn exec_block(
    ctx: &impl ChainWorkerContext,
    bridge_params: BridgeParams,
    block_commitment: &OLBlockCommitment,
) -> WorkerResult<()> {
    let blkid = block_commitment.blkid();
    debug!(%blkid, "Trying to execute block");

    // Fetch block and parent context
    let (block, parent_header, parent_commitment) = fetch_block_with_parent(ctx, block_commitment)?;

    // Execute STF and get output and new state
    let (output, new_state) = execute_stf(
        ctx,
        bridge_params,
        &block,
        parent_header.as_ref(),
        parent_commitment,
    )?;

    let is_terminal = block.header().is_terminal();
    debug!(slot=%block.header().slot(), is_terminal, "Checking if block is terminal");

    if is_terminal {
        // Persist results (including the full state) before the terminal
        // post-ops; see the doc comment above.
        persist_execution_output(ctx, &block, *block_commitment, &output, new_state.clone())?;

        // Handle epoch terminal
        // TODO(STR-3673): the epoch commitment seems to be sent to the
        // receiver for each block at the moment. Ideally we would do it just
        // here.
        handle_terminal_block_exec_post_ops(ctx, &block, &output, &new_state)?;
    } else {
        // Persist results (including the full state)
        persist_execution_output(ctx, &block, *block_commitment, &output, new_state)?;
    }

    Ok(())
}

/// Fetches a block and its parent header from the context.
///
/// Returns the block, optional parent header, and parent commitment.
fn fetch_block_with_parent(
    ctx: &impl ChainWorkerContext,
    block_commitment: &OLBlockCommitment,
) -> WorkerResult<(OLBlock, Option<OLBlockHeader>, OLBlockCommitment)> {
    let blkid = block_commitment.blkid();

    let block = ctx
        .fetch_block(blkid)?
        .ok_or(WorkerError::MissingOLBlock(*blkid))?;

    let parent_blkid = block.header().parent_blkid();
    let parent_commitment = if parent_blkid.is_null() {
        OLBlockCommitment::null()
    } else {
        // Parent slot is the block's slot - 1.
        let parent_slot = block.header().slot().saturating_sub(1);
        OLBlockCommitment::new(parent_slot, *parent_blkid)
    };

    let parent_header = if parent_commitment.is_null() {
        None
    } else {
        Some(
            ctx.fetch_header(parent_commitment.blkid())?
                .ok_or(WorkerError::MissingOLBlock(*parent_commitment.blkid()))?,
        )
    };

    Ok((block, parent_header, parent_commitment))
}

/// Executes the STF on a block and returns the execution output.
///
/// This fetches parent state, builds the state stack, runs verification,
/// and extracts the resulting write batch and indexer writes.
#[instrument(
    skip_all,
    fields(
        slot = block.header().slot(),
        epoch = block.header().epoch(),
        is_terminal = block.header().is_terminal(),
        %parent_commitment,
    ),
    err,
)]
fn execute_stf(
    ctx: &impl ChainWorkerContext,
    bridge_params: BridgeParams,
    block: &OLBlock,
    parent_header: Option<&OLBlockHeader>,
    parent_commitment: OLBlockCommitment,
) -> WorkerResult<(OLBlockExecutionOutput, OLState)> {
    // Fetch parent state and wrap in MemoryStateBaseLayer for IStateAccessor
    let parent_state_raw = ctx
        .fetch_ol_state(parent_commitment)?
        .ok_or(WorkerError::MissingPreState(parent_commitment))?;
    let parent_state = MemoryStateBaseLayer::new(parent_state_raw);

    // Execute and extract outputs
    let (write_batch, indexer_writes, logs) =
        run_stf_verification(&parent_state, block, parent_header, bridge_params)?;

    // Apply write batch to parent state to get new state
    let mut new_state = parent_state;
    new_state
        .apply_write_batch(write_batch.clone())
        .map_err(|source| WorkerError::ApplyWriteBatch {
            commitment: parent_commitment,
            source,
        })?;
    let new_state = new_state.into_inner();

    // Use the state root from the header (verify_block validated it).
    // Note: logs are validated internally by verify_block via the logs_root commitment.
    let computed_state_root = *block.header().state_root();

    Ok((
        OLBlockExecutionOutput::new(computed_state_root, write_batch, indexer_writes, logs),
        new_state,
    ))
}

/// Persists the execution output and state to storage.
///
/// Handles crash-restart cases like indexing already applied or wrong indexing applied
/// gracefully.
fn persist_execution_output(
    ctx: &impl ChainWorkerContext,
    block: &OLBlock,
    block_commitment: OLBlockCommitment,
    output: &OLBlockExecutionOutput,
    new_state: OLState,
) -> WorkerResult<()> {
    match ctx.store_block_output(block, block_commitment, output) {
        Ok(()) => {}
        Err(WorkerError::Database(DbError::BlockIndexingConflict {
            attempted,
            last_applied,
            ..
        })) if attempted == block_commitment && last_applied == block_commitment => {
            debug!(
                %block_commitment,
                "block indexing already applied for this exact block; \
                 treating as crash-restart retry and continuing persist"
            );
        }
        Err(e) => return Err(e),
    }

    ctx.store_toplevel_state(block_commitment, new_state)?;

    Ok(())
}

/// Takes the block and post-state and inserts database entries to reflect
/// the epoch being finished on-chain.
///
/// Expects the terminal block's own output to be persisted already: the merge
/// reads every block's write batch including the terminal block's, and epoch
/// finalization stamps the epoch commitment onto an existing indexing row.
///
/// The summary is stored last, after the epoch data merge: a stored summary
/// (and the notification it emits) indicates the whole epoch finalization
/// persisted. A failure anywhere earlier leaves no summary behind for a
/// block that may then be rejected.
fn handle_terminal_block_exec_post_ops(
    ctx: &impl ChainWorkerContext,
    block: &OLBlock,
    last_block_output: &OLBlockExecutionOutput,
    new_state: &OLState,
) -> WorkerResult<()> {
    let completed_epoch = block.header().epoch();

    let prev_terminal = get_prev_terminal(ctx, completed_epoch)?;
    let summary = build_epoch_summary(block.header(), last_block_output, new_state, prev_terminal);

    // Merge the epoch's write batches into the terminal state.
    ctx.merge_epoch_data(&summary)?;

    debug!(?summary, "completed chain epoch");
    ctx.store_summary(summary)?;

    Ok(())
}

/// Gets the terminal commitment of the epoch before `cur_epoch`.
///
/// Expects `cur_epoch > 0`; the chain worker executes blocks from slot 1, so
/// the previous epoch's terminal is always already stored.
pub(crate) fn get_prev_terminal(
    ctx: &impl ChainWorkerContext,
    cur_epoch: Epoch,
) -> WorkerResult<OLBlockCommitment> {
    if cur_epoch == 0 {
        return Err(WorkerError::Unexpected(
            "received prev terminal request for unexpected epoch 0".to_string(),
        ));
    }
    let target_epoch = cur_epoch - 1;
    ctx.fetch_canonical_epoch_summary_at(target_epoch)?
        .map(|s| *s.terminal())
        .ok_or(WorkerError::MissingSummaryForEpoch(target_epoch))
}

/// Reconstructs an epoch's OL state from its checkpoint, persisting nothing.
pub(crate) fn apply_checkpoint_epoch(
    ctx: &impl ChainWorkerContext,
    epoch: EpochCommitment,
) -> WorkerResult<AppliedEpochArtifacts> {
    // Epoch 0 is genesis-initialized on every node, never checkpoint-applied.
    if epoch.epoch() == 0 {
        return Err(WorkerError::CannotApplyGenesisEpoch);
    }

    let payload = ctx
        .fetch_checkpoint_payload(&epoch)?
        .ok_or(WorkerError::MissingCheckpointPayload(epoch))?;

    // Cross-check the payload tip against the requested epoch. Defends against
    // a key→value mismatch from a future storage backend / writer bug.
    let tip = payload.new_tip();
    if tip.epoch != epoch.epoch() || tip.l2_commitment() != &epoch.to_block_commitment() {
        return Err(WorkerError::CheckpointTipMismatch {
            epoch: epoch.epoch(),
            payload_epoch: tip.epoch,
            payload_l2: *tip.l2_commitment(),
        });
    }

    // Fetch previous terminal state.
    let prev_terminal = get_prev_terminal(ctx, epoch.epoch())?;
    let base_state_raw = ctx
        .fetch_ol_state(prev_terminal)?
        .ok_or(WorkerError::MissingPreState(prev_terminal))?;
    let base_state = MemoryStateBaseLayer::new(base_state_raw);

    let sidecar = payload.sidecar();
    let terminal = *tip.l2_commitment();
    let da_payload = decode_ol_da_payload_bytes(sidecar.ol_state_diff()).map_err(|source| {
        WorkerError::DaPayloadDecode {
            epoch: epoch.epoch(),
            source,
        }
    })?;
    let (manifests, epoch_info) =
        assemble_da_inputs(ctx, epoch, &base_state, sidecar, tip, prev_terminal)?;

    // The sidecar carries logs in its own wire type; convert to the OL chain-types
    // `OLLog` so the reconstructed epoch output mirrors the per-block path.
    let ol_logs: Vec<OLLog> = sidecar
        .ol_logs()
        .iter()
        .map(|log| OLLog::new(log.account_serial(), log.payload().to_vec()))
        .collect();

    // Read each snark account's seqno and inbox cursor as of the previous
    // epoch's terminal state. The set of affected accounts is inferred from
    // `ol_logs`, which records the changes made during the epoch.
    let pre_cursors = collect_pre_snark_account_cursors(&base_state, &ol_logs)?;

    // Reconstruct: wrap the base state in the write-tracking + indexer stack,
    // run apply_da_epoch, then extract the batch and indexer writes.
    let tracking_state = WriteTrackingState::new_empty(&base_state);
    let mut indexer_state = IndexerState::new(tracking_state);
    apply_da_epoch::<_, OLDaSchemeV1>(&mut indexer_state, &epoch_info, da_payload, &manifests)?;
    let indexer_state_root =
        indexer_state
            .compute_state_root()
            .map_err(|source| WorkerError::StateRootCompute {
                epoch: epoch.epoch(),
                stage: "indexer",
                source,
            })?;

    // Replace the DA-collapsed snark records (one per account) with per-update
    // records derived from the epoch's `ol_logs`.
    let recons_data =
        reconstruct_snark_acct_update_records(&indexer_state, &ol_logs, &pre_cursors)?;
    let derived_seqnos: HashMap<AccountSerial, Seqno> = recons_data
        .acct_cursors
        .iter()
        .map(|(&serial, cursor)| (serial, cursor.seqno))
        .collect();

    let (tracking_state, mut indexer_writes) = indexer_state.into_parts();
    let write_batch: WriteBatch<OLAccountState> = tracking_state.into_batch();

    // Apply the batch onto the base state to get the reconstructed state.
    let mut new_state = base_state;
    new_state
        .apply_write_batch(write_batch.clone())
        .map_err(|source| WorkerError::ApplyWriteBatch {
            commitment: prev_terminal,
            source,
        })?;

    verify_snark_seqno_invariant(&new_state, &derived_seqnos)?;

    indexer_writes.set_snark_acct_state_updates(recons_data.updates);
    let final_state_root =
        new_state
            .compute_state_root()
            .map_err(|source| WorkerError::StateRootCompute {
                epoch: epoch.epoch(),
                stage: "final",
                source,
            })?;

    // Now verify.
    verify_reconstruction(
        epoch,
        tip,
        sidecar,
        terminal,
        indexer_state_root,
        final_state_root,
    )?;

    let new_state = new_state.into_inner();

    // L1 info comes from the reconstructed post-state's epochal state, the same
    // source FCM's `build_epoch_summary` uses.
    let epoch_state = new_state.epoch_state();
    let new_l1 = L1BlockCommitment::new(epoch_state.last_l1_height(), *epoch_state.last_l1_blkid());

    let summary = EpochSummary::new(
        epoch.epoch(),
        terminal,
        prev_terminal,
        new_l1,
        final_state_root,
    );
    let output =
        OLBlockExecutionOutput::new(final_state_root, write_batch, indexer_writes, ol_logs);

    Ok(AppliedEpochArtifacts {
        terminal,
        new_state,
        summary,
        output,
    })
}

/// Validates the epoch's L1 range and builds the manifest list and [`EpochInfo`]
/// used to drive [`apply_da_epoch`].
///
/// The manifests cover the whole epoch, so this intentionally uses a [`Vec`]
/// instead of the per-block `OLAsmManifestContainer`. The range is still capped
/// by the epoch manifest limit before any manifests are fetched.
fn assemble_da_inputs(
    ctx: &impl ChainWorkerContext,
    epoch: EpochCommitment,
    base_state: &MemoryStateBaseLayer,
    sidecar: &CheckpointSidecar,
    tip: &CheckpointTip,
    prev_terminal: OLBlockCommitment,
) -> WorkerResult<(Vec<AsmManifest>, EpochInfo)> {
    let base_l1_height = base_state.last_l1_height();
    let to_height = tip.l1_height();
    if to_height < base_l1_height {
        return Err(WorkerError::L1RangeInverted {
            epoch: epoch.epoch(),
            from: base_l1_height.saturating_add(1),
            to: to_height,
        });
    }
    let manifests = if to_height == base_l1_height {
        Vec::new()
    } else {
        let from_height = base_l1_height.checked_add(1).ok_or_else(|| {
            WorkerError::Unexpected(format!(
                "L1 height overflow at base state for {epoch}: {base_l1_height}",
            ))
        })?;
        let range_len = to_height - from_height + 1;
        if range_len > MAX_SEALING_MANIFEST_COUNT as u32 {
            return Err(WorkerError::L1RangeTooLarge {
                epoch: epoch.epoch(),
                len: range_len,
                max: MAX_SEALING_MANIFEST_COUNT as u32,
            });
        }
        ctx.fetch_l1_manifests(from_height, to_height)?
    };

    let terminal = tip.l2_commitment();
    let terminal_info = BlockInfo::new(
        sidecar.terminal_header_complement().timestamp(),
        terminal.slot(),
        epoch.epoch(),
    );
    let epoch_info = EpochInfo::new(terminal_info, prev_terminal);

    Ok((manifests, epoch_info))
}

/// Cross-checks the reconstructed epoch against the checkpoint's bindings.
fn verify_reconstruction(
    epoch: EpochCommitment,
    tip: &CheckpointTip,
    sidecar: &CheckpointSidecar,
    terminal: OLBlockCommitment,
    indexer_state_root: Buf32,
    final_state_root: Buf32,
) -> WorkerResult<()> {
    if final_state_root != indexer_state_root {
        error!(
            %epoch, %indexer_state_root, %final_state_root,
            payload_tip_epoch = tip.epoch,
            payload_tip_l1 = tip.l1_height(),
            payload_tip_l2 = ?tip.l2_commitment(),
            "epoch reconstruction state root divergence",
        );
        return Err(WorkerError::StateRootDivergence {
            epoch: epoch.epoch(),
            indexer_root: indexer_state_root,
            final_root: final_state_root,
        });
    }

    let mut terminal_flags = BlockFlags::zero();
    terminal_flags.set_is_terminal(true);
    let complement = sidecar.terminal_header_complement();
    let reconstructed_header = OLBlockHeader::new(
        complement.timestamp(),
        terminal_flags,
        terminal.slot(),
        epoch.epoch(),
        *complement.parent_blkid(),
        *complement.body_root(),
        final_state_root,
        *complement.logs_root(),
    );
    let reconstructed_blkid = reconstructed_header.compute_blkid();
    if reconstructed_blkid != *terminal.blkid() {
        error!(
            %epoch,
            expected_blkid = %terminal.blkid(),
            %reconstructed_blkid,
            "terminal header reconstruction blkid mismatch",
        );
        return Err(WorkerError::TerminalBlkidMismatch {
            epoch: epoch.epoch(),
            expected: *terminal.blkid(),
            reconstructed: reconstructed_blkid,
        });
    }

    Ok(())
}

/// Asserts that the seqno derived per-log matches the post-state seqno reached via
/// [`apply_da_epoch`] + write batch.
fn verify_snark_seqno_invariant<S: IStateAccessor>(
    state: &S,
    derived_seqnos: &HashMap<AccountSerial, Seqno>,
) -> WorkerResult<()> {
    for (&serial, &derived) in derived_seqnos {
        let account_id = state
            .find_account_id_by_serial(serial)
            .map_err(acct_read_err("post-state serial lookup"))?
            .ok_or(WorkerError::UnknownAccountSerial(serial))?;
        let post = get_snark_acct(state, account_id)
            .map(|s| s.seqno())
            .map_err(acct_read_err("post-state as_snark_account"))?;
        if post != derived {
            error!(
                ?serial, %account_id, ?derived, ?post,
                "snark seqno invariant violated",
            );
            return Err(WorkerError::SnarkSeqnoMismatch {
                serial,
                account_id,
                derived,
                post,
            });
        }
    }
    Ok(())
}

/// Reads each affected snark account's pre-epoch cursor state.
///
/// The account set is inferred from `ol_logs` (a record of changes made during
/// the epoch).
fn collect_pre_snark_account_cursors<S: IStateAccessor>(
    base_state: &S,
    ol_logs: &[OLLog],
) -> WorkerResult<SnarkAccountCursors> {
    let mut pre_cursors = HashMap::new();
    for log in ol_logs {
        let serial = log.account_serial();
        if MsgRef::try_from(log.payload())?.ty() != SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID {
            continue;
        }
        if pre_cursors.contains_key(&serial) {
            continue;
        }
        let cursor = get_snark_acct_cursor(base_state, serial)?;
        pre_cursors.insert(serial, cursor);
    }
    Ok(pre_cursors)
}

/// Gets account state and constructs `SnarkAccountCursor` from the state defaulting to zero valued
/// cursors.
fn get_snark_acct_cursor<S: IStateAccessor>(
    base_state: &S,
    serial: AccountSerial,
) -> WorkerResult<SnarkAccountCursor> {
    let cursor = match base_state
        .find_account_id_by_serial(serial)
        .map_err(acct_read_err("pre-state serial lookup"))?
    {
        None => SnarkAccountCursor::default(),
        Some(account_id) => match get_snark_acct(base_state, account_id) {
            Ok(snacct) => SnarkAccountCursor {
                seqno: snacct.seqno(),
                next_inbox_msg_idx: snacct.next_inbox_msg_idx(),
            },
            Err(StateError::MissingAccount(_)) => SnarkAccountCursor::default(),
            Err(source) => {
                return Err(acct_read_err("pre-state account read")(source));
            }
        },
    };
    Ok(cursor)
}

/// Reconstructs per-update snark index records from the epoch's OL logs.
fn reconstruct_snark_acct_update_records<S: IStateAccessor>(
    state: &S,
    ol_logs: &[OLLog],
    pre_cursors: &SnarkAccountCursors,
) -> WorkerResult<ReconstructedSnarkData> {
    let mut acct_cursors: SnarkAccountCursors = HashMap::new();
    let mut updates = Vec::with_capacity(ol_logs.len());
    // Index of each account's last update, so we can stamp the recoverable
    // post-epoch root onto it after the walk.
    let mut acct_last_record_idx: HashMap<AccountId, usize> = HashMap::new();

    for log in ol_logs {
        let serial = log.account_serial();
        let msg = MsgRef::try_from(log.payload())?;
        if msg.ty() != SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID {
            info!(
                ?serial,
                "non-snark update log in checkpoint logs. Ignoring."
            );
            continue;
        }
        let log_data = SnarkAccountUpdateLogData::try_decode_log(&msg)?;

        let account_id = state
            .find_account_id_by_serial(serial)
            .map_err(acct_read_err("post-state serial lookup"))?
            .ok_or(WorkerError::UnknownAccountSerial(serial))?;

        let cur = acct_cursors.entry(serial).or_insert_with(|| {
            pre_cursors
                .get(&serial)
                .copied()
                .unwrap_or(SnarkAccountCursor::default())
        });
        let prev_next_read_idx = cur.next_inbox_msg_idx;
        cur.seqno = cur.seqno.incr();
        cur.next_inbox_msg_idx = log_data.new_msg_idx;
        let seqno = cur.seqno;

        // Intermediate per-update roots are not in checkpoint logs; the terminal
        // update is patched below with the recoverable post-epoch root.
        acct_last_record_idx.insert(account_id, updates.len());
        updates.push(SnarkAcctStateUpdate::new(
            account_id,
            None,
            prev_next_read_idx,
            log_data.new_msg_idx,
            seqno,
        ));
    }

    // The post-DA state holds each account's final inner state root, which is
    // the terminal update's post-state root. Earlier updates stay `None`.
    for (account_id, idx) in acct_last_record_idx {
        let root = get_snark_acct(state, account_id)
            .map(|s| s.inner_state_root())
            .map_err(acct_read_err("post epoch snark state root"))?;
        updates[idx].set_state(Some(root));
    }

    Ok(ReconstructedSnarkData {
        updates,
        acct_cursors,
    })
}

/// Gets corresponding snark account for given account id.
fn get_snark_acct<S: IStateAccessor>(
    state: &S,
    account_id: AccountId,
) -> StateResult<&<S::AccountState as IAccountState>::SnarkAccountState> {
    let Some(acct) = state.get_account_state(account_id)? else {
        return Err(StateError::MissingAccount(account_id));
    };

    acct.as_snark_account()
}

/// Builds a [`WorkerError::AccountStateRead`] closure tagged with `stage`.
fn acct_read_err(stage: &'static str) -> impl FnOnce(StateError) -> WorkerError {
    move |source| WorkerError::AccountStateRead { stage, source }
}

/// The values produced by reconstructing one epoch from its checkpoint,
/// before persistence.
#[derive(Debug)]
pub(crate) struct AppliedEpochArtifacts {
    /// Terminal block commitment of the epoch.
    pub(crate) terminal: OLBlockCommitment,
    /// Reconstructed post-epoch toplevel state.
    pub(crate) new_state: OLState,
    /// Epoch summary built from the reconstructed state.
    pub(crate) summary: EpochSummary,
    /// Execution output (state root, write batch, indexer writes).
    pub(crate) output: OLBlockExecutionOutput,
}

impl ServiceState for ChainWorkerServiceState {
    fn name(&self) -> &str {
        "chain_worker"
    }
}

/// Runs the STF verification on a block.
///
/// This is a pure function that builds the state stack, executes the STF, and returns the write
/// batch, indexer writes, and emitted logs.
#[instrument(
    skip_all,
    fields(
        slot = block.header().slot(),
        epoch = block.header().epoch(),
        is_terminal = block.header().is_terminal(),
    ),
    err,
)]
fn run_stf_verification(
    parent_state: &MemoryStateBaseLayer,
    block: &OLBlock,
    parent_header: Option<&OLBlockHeader>,
    bridge_params: BridgeParams,
) -> WorkerResult<(WriteBatch<OLAccountState>, IndexerWrites, Vec<OLLog>)> {
    // Build the state stack: IndexerState<WriteTrackingState<&MemoryStateBaseLayer>>
    let tracking_state = WriteTrackingState::new_empty(parent_state);
    let mut indexer_state = IndexerState::new(tracking_state);

    let logs = verify_block(
        &mut indexer_state,
        block.header(),
        parent_header,
        block.body(),
        bridge_params,
    )?;

    // Extract outputs
    let (tracking_state, indexer_writes) = indexer_state.into_parts();
    let write_batch: WriteBatch<OLAccountState> = tracking_state.into_batch();

    Ok((write_batch, indexer_writes, logs))
}

/// Builds the [`EpochSummary`] for a completed epoch from the terminal block,
/// its execution output, and the post-state.
///
/// L1 info is sourced from the post-state's epochal state rather than the
/// write batch, since the batch only carries diffs and may not contain a
/// `last_l1_*` update if the terminal block introduced no new manifests.
fn build_epoch_summary(
    block_header: &OLBlockHeader,
    last_block_output: &OLBlockExecutionOutput,
    new_state: &OLState,
    prev_terminal: OLBlockCommitment,
) -> EpochSummary {
    let completed_epoch = block_header.epoch();
    let terminal = OLBlockCommitment::new(block_header.slot(), block_header.compute_blkid());

    // Read L1 info from the post-state. The write batch only stores diffs, so it
    // may not contain a last_l1_* update if the terminal block had no new manifests.
    let epoch_state = new_state.epoch_state();
    let new_l1_block =
        L1BlockCommitment::new(epoch_state.last_l1_height(), *epoch_state.last_l1_blkid());

    let epoch_final_state = *last_block_output.computed_state_root();

    EpochSummary::new(
        completed_epoch,
        terminal,
        prev_terminal,
        new_l1_block,
        epoch_final_state,
    )
}

#[cfg(test)]
mod tests {
    use strata_acct_types::Hash;
    use strata_identifiers::{Buf32, L1BlockCommitment, L1BlockId, L1Height, OLBlockId};
    use strata_ol_chain_types::{BlockFlags, OLBlockHeader};
    use strata_ol_state_support_types::IndexerWrites;
    use strata_ol_state_types::{
        OLAccountState, WriteBatch, test_utils::create_test_genesis_state,
    };

    use super::*;
    use crate::OLBlockExecutionOutput;

    /// Regression test for the panic
    /// `terminal block must have L1 height in write batch`.
    ///
    /// When a terminal block does not introduce any new ASM manifests, its
    /// [`WriteBatch`] does not contain `last_l1_height` / `last_l1_blkid`
    /// updates. The summary builder must therefore read L1 info from the
    /// post-state rather than the write batch.
    #[test]
    fn test_handle_write_batch_without_last_l1_change() {
        // Set up a post-state with a known L1 commitment, simulating L1 progress
        // recorded in earlier (non-terminal) blocks of the epoch.
        let mut new_state = create_test_genesis_state();
        let expected_height = L1Height::from(1234u32);
        let expected_blkid = L1BlockId::from(Buf32::from([7u8; 32]));
        let mut setup_batch: WriteBatch<OLAccountState> = WriteBatch::default();
        setup_batch.epochal_writes_mut().last_l1_height = Some(expected_height);
        setup_batch.epochal_writes_mut().last_l1_blkid = Some(expected_blkid);
        new_state
            .apply_write_batch(setup_batch)
            .expect("apply setup batch");

        // Terminal block's write batch carries no `last_l1_*` update — this is
        // the case that previously panicked.
        let terminal_batch: WriteBatch<OLAccountState> = WriteBatch::default();
        assert!(terminal_batch.epochal_writes().last_l1_height.is_none());
        assert!(terminal_batch.epochal_writes().last_l1_blkid.is_none());

        let state_root = Buf32::from([9u8; 32]);
        let output =
            OLBlockExecutionOutput::new(state_root, terminal_batch, IndexerWrites::new(), vec![]);

        let mut flags = BlockFlags::zero();
        flags.set_is_terminal(true);
        let header = OLBlockHeader::new(
            0,
            flags,
            10,
            5,
            OLBlockId::from(Buf32::zero()),
            Buf32::zero(),
            state_root,
            Buf32::zero(),
        );

        let summary = build_epoch_summary(&header, &output, &new_state, OLBlockCommitment::null());

        assert_eq!(
            summary.new_l1(),
            &L1BlockCommitment::new(expected_height, expected_blkid),
            "L1 commitment must come from post-state, not from the empty write batch"
        );
        assert_eq!(summary.epoch(), 5);
        assert_eq!(summary.final_state(), &state_root);
        assert_eq!(summary.prev_terminal(), &OLBlockCommitment::null());
    }

    /// Checkpoint reconstruction can only recover the terminal per-account root
    /// (= post-epoch state root). Earlier updates in a multi-update epoch stay
    /// `None`; the terminal update is stamped with the account's final root.
    #[test]
    fn rebuild_snark_records_stamps_terminal_root_only() {
        use strata_acct_types::BitcoinAmount;
        use strata_ledger_types::{IStateAccessorMut, NewAccountData, NewAccountTypeState};
        use strata_ol_state_support_types::MemoryStateBaseLayer;
        use strata_predicate::PredicateKey;

        let account_id = AccountId::from([7u8; 32]);
        let final_root = Hash::from([9u8; 32]);

        let mut state = MemoryStateBaseLayer::new(create_test_genesis_state());
        let serial = state
            .create_new_account(
                account_id,
                NewAccountData::new(
                    BitcoinAmount::zero(),
                    NewAccountTypeState::Snark {
                        update_vk: PredicateKey::always_accept(),
                        initial_state_root: final_root,
                    },
                ),
            )
            .expect("create snark account");

        // Two updates to the same account in one epoch (cursors advance 0->1->2).
        let log = |new_idx: u64| {
            let data = SnarkAccountUpdateLogData::new(new_idx, vec![0xaa]).unwrap();
            OLLog::new(serial, data.encode_log().unwrap())
        };
        let logs = vec![log(1), log(2)];

        let ReconstructedSnarkData {
            updates: records, ..
        } = reconstruct_snark_acct_update_records(&state, &logs, &HashMap::new()).expect("rebuild");

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].state(), None, "earlier update root unavailable");
        assert_eq!(
            records[1].state(),
            Some(final_root),
            "terminal update carries post-epoch root"
        );
    }
}
