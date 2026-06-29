//! Block assembly logic.

use std::{
    slice,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use strata_bridge_params::BridgeParams;
use strata_config::SequencerConfig;
use strata_db_types::errors::DbError;
use strata_identifiers::{Epoch, OLBlockCommitment, OLTxId, Slot};
use strata_ledger_types::{
    AccProofCheck, IAccountState, ISnarkAccountState, IStateAccessor, TxProofIndexer, *,
};
use strata_ol_chain_types_new::*;
use strata_ol_mempool::MempoolTxInvalidReason;
use strata_ol_state_support_types::{DaAccumulatingState, WriteTrackingState};
use strata_ol_state_types::{MAX_PENDING_ASM_LOGS, WriteBatch};
use strata_ol_stf::*;
use strata_snark_acct_types as _;
use tracing::{debug, error, warn};

use crate::{
    AccumulatorProofGenerator, BlockAssemblyResult, BlockAssemblyStateAccess, MempoolProvider,
    checkpoint_size::LogMetrics,
    context::BlockAssemblyAnchorContext,
    epoch_sealing::{
        EpochSealingLimitAction, EpochSealingLimitVerdict, EpochSealingPolicy,
        EpochSealingResourceStats,
    },
    error::BlockAssemblyError,
    resource_state::{AccumulatedDaData, EpochResourceState},
    types::{BlockGenerationConfig, BlockTemplateResult, FailedMempoolTx, FullBlockTemplate},
};

/// Output from processing transactions during block assembly.
struct ProcessTransactionsOutput<S: IStateAccessor> {
    /// Transactions that passed validation and execution
    successful_txs: Vec<OLTransaction>,
    /// Transactions that failed during block assembly.
    failed_txs: Vec<FailedMempoolTx>,
    /// Accumulated write batch after processing all transactions.
    accumulated_batch: WriteBatch<S::AccountState>,
    /// Accumulated da data after processing all transactions.
    accumulated_da: AccumulatedDaData,
    /// Non-cadence limits requesting an epoch seal, if any.
    sealing_limit_verdict: EpochSealingLimitVerdict,
}

/// Inputs consumed while finalizing a block template.
struct BuildBlockTemplateInput<A> {
    accumulated_batch: WriteBatch<A>,
    output_buffer: ExecOutputBuffer,
    successful_txs: Vec<OLTransaction>,
    is_terminal: bool,
    manifest_container: Option<OLAsmManifestContainer>,
}

/// ASM manifests selected for the candidate OL block.
///
/// The selected manifests are still raw manifests; the caller wraps them in an
/// [`OLAsmManifestContainer`] when the sequence is non-empty.
struct SelectedAsmManifests {
    manifests: Vec<AsmManifest>,
    sealing_limit_verdict: EpochSealingLimitVerdict,
}

/// Maps an [`ExecError`] to a [`MempoolTxInvalidReason`].
///
/// Determines how block assembly reports tx failures to mempool:
/// - `Invalid` → tx expired or invalid according to consensus rules
/// - `Failed` → tx failed due to transient issues
fn stf_exec_error_to_mempool_reason(err: &ExecError) -> MempoolTxInvalidReason {
    match err {
        // Expired: tx will never succeed
        ExecError::TransactionExpired(_, _) => MempoolTxInvalidReason::Invalid,

        // Protocol violations: deterministically invalid
        ExecError::SignatureInvalid(_)
        | ExecError::UnknownAccount(_)
        | ExecError::IncorrectTxTargetType
        | ExecError::Codec(_)
        | ExecError::Acct(_)
        | ExecError::LogsOverflow { .. } => MempoolTxInvalidReason::Invalid,

        // May succeed in future blocks
        ExecError::TransactionNotMature(_, _)
        | ExecError::TxConditionCheckFailed
        | ExecError::BalanceUnderflow
        | ExecError::InsufficientAccountBalance { id: _, need: _ } => {
            MempoolTxInvalidReason::Failed
        }

        // Block-level errors shouldn't occur in tx processing
        _ => MempoolTxInvalidReason::Failed,
    }
}

/// Maps a [`BlockAssemblyError`] to a [`MempoolTxInvalidReason`].
fn block_assembly_error_to_mempool_reason(err: &BlockAssemblyError) -> MempoolTxInvalidReason {
    match err {
        // Tx claimed invalid accumulator proof - permanently invalid
        BlockAssemblyError::InvalidAccumulatorClaim(_)
        | BlockAssemblyError::Acct(_)
        | BlockAssemblyError::State(_)
        | BlockAssemblyError::L1BlockRefHashMismatch { .. }
        | BlockAssemblyError::InboxEntryHashMismatch { .. }
        | BlockAssemblyError::AccountNotFound(_)
        | BlockAssemblyError::InboxProofCountMismatch { .. } => MempoolTxInvalidReason::Invalid,

        // Pre-validation via TxProofIndexer: delegate to STF error classification.
        BlockAssemblyError::SnarkUpdatePreValidation(exec_err) => {
            stf_exec_error_to_mempool_reason(exec_err)
        }

        BlockAssemblyError::Db(db_err) => match db_err {
            DbError::MmrLeafNotFound(_)
            | DbError::MmrLeafNotFoundForAccount(_, _)
            | DbError::MmrNodeNotFound(_)
            | DbError::MmrInvalidRange { .. }
            | DbError::MmrPayloadNotFound(_)
            | DbError::MmrPositionOutOfBounds { .. } => MempoolTxInvalidReason::Invalid,
            // MmrIndexOutOfRange is transient: the tx may reference manifests
            // that the OL state hasn't caught up to yet (ASM manifests MMR is
            // only updated at epoch boundaries). Keep in mempool for retry.
            DbError::MmrIndexOutOfRange { .. } => MempoolTxInvalidReason::Failed,
            DbError::MmrPreconditionFailed { .. } => MempoolTxInvalidReason::Failed,
            _ => MempoolTxInvalidReason::Failed,
        },

        // Block assembly internal errors (not consensus-related).
        BlockAssemblyError::BlockConstruction(_)
        | BlockAssemblyError::ChainTypes(_)
        | BlockAssemblyError::InvalidRange { .. }
        | BlockAssemblyError::InvalidSignature(_)
        | BlockAssemblyError::Mempool(_)
        | BlockAssemblyError::StateProvider(_)
        | BlockAssemblyError::NoPendingTemplateForParent(_)
        | BlockAssemblyError::TemplateAlreadyCompletedForParent { .. }
        | BlockAssemblyError::Other(_)
        | BlockAssemblyError::RequestChannelClosed
        | BlockAssemblyError::ResponseChannelClosed
        | BlockAssemblyError::UnknownTemplateId(_)
        | BlockAssemblyError::TimestampTooEarly(_)
        | BlockAssemblyError::BlockNotFound(_)
        | BlockAssemblyError::ParentStateNotFound(_)
        | BlockAssemblyError::GenesisEpochNoBoundary
        | BlockAssemblyError::InvalidEpochBoundary { .. }
        | BlockAssemblyError::EpochBoundaryStateNotFound(_)
        | BlockAssemblyError::TooManyClaims
        | BlockAssemblyError::CannotBuildGenesis => MempoolTxInvalidReason::Failed,
    }
}

/// Output from block construction containing the template, failed transactions, and final state.
pub(crate) struct ConstructBlockOutput<S> {
    /// The constructed block template.
    pub(crate) template: FullBlockTemplate,
    /// Transactions that failed during block assembly.
    pub(crate) failed_txs: Vec<FailedMempoolTx>,
    /// The post state after applying all transactions.
    // Used by tests to chain blocks without re-executing through STF.
    #[cfg_attr(
        all(not(test), not(feature = "test-utils")),
        expect(dead_code, reason = "only used by tests")
    )]
    pub(crate) post_state: S,
    /// Resource state for the constructed block's epoch.
    pub(crate) resource_state: EpochResourceState,
}

/// Generate a block template from the given configuration.
///
/// Fetches transactions from the mempool, generates accumulator proofs, validates execution
/// with per-transaction staging, and constructs a complete block template.
///
/// Transactions that fail proof generation or execution are returned in `failed_txs`.
/// Reporting those failures to the mempool is handled by the service layer.
///
/// Returns a [`BlockTemplateResult`] containing both the generated template and
/// any transactions that failed validation during assembly.
pub(crate) async fn generate_block_template_inner<C, E>(
    ctx: &C,
    epoch_sealing_policy: &E,
    sequencer_config: &SequencerConfig,
    block_generation_config: BlockGenerationConfig,
    resource_state_before_block: EpochResourceState,
    bridge_params: BridgeParams,
) -> BlockAssemblyResult<BlockTemplateResult>
where
    C: BlockAssemblyAnchorContext + AccumulatorProofGenerator + MempoolProvider,
    C::State: BlockAssemblyStateAccess,
    E: EpochSealingPolicy,
    <<C::State as IStateAccessor>::AccountState as IAccountStateMut>::SnarkAccountStateMut: Clone,
{
    let max_txs_per_block = sequencer_config.max_txs_per_block;

    // 1. Fetch parent state
    let parent_commitment = block_generation_config.parent_block_commitment();
    assert!(
        !parent_commitment.is_null(),
        "generate_block_template_inner called with null parent - genesis must be built via init_ol_genesis"
    );

    let parent_state = ctx
        .fetch_state_for_tip(parent_commitment)
        .await?
        .ok_or(BlockAssemblyError::ParentStateNotFound(parent_commitment))?;

    // 2. Calculate next slot and epoch
    let (block_slot, block_epoch) =
        calculate_block_slot_and_epoch(&parent_commitment, parent_state.as_ref());

    // 3. Get transactions from mempool
    let mempool_txs = MempoolProvider::get_transactions(ctx, max_txs_per_block).await?;

    // 4. Construct block using the sealing policy decision and manifest fetch gate.
    let output = construct_block(
        ctx,
        epoch_sealing_policy,
        &block_generation_config,
        parent_state,
        block_slot,
        block_epoch,
        mempool_txs,
        resource_state_before_block,
        bridge_params,
    )
    .await?;

    Ok(BlockTemplateResult::new(
        output.template,
        output.failed_txs,
        output.resource_state,
    ))
}

/// Calculates the next slot and epoch based on parent commitment and state.
///
/// Returns `(parent_slot + 1, parent_state.cur_epoch())`
///
/// Note: parent_state.cur_epoch() already reflects the correct epoch:
/// - If parent was non-terminal: epoch stays the same
/// - If parent was terminal: epoch was advanced during manifest processing
///
/// # Panics
/// Panics if `parent_commitment` is null. Genesis blocks must be created via
/// `init_ol_genesis`, not through block assembly.
pub(crate) fn calculate_block_slot_and_epoch<S: IStateAccessor>(
    parent_commitment: &OLBlockCommitment,
    parent_state: &S,
) -> (Slot, Epoch) {
    assert!(
        !parent_commitment.is_null(),
        "Cannot calculate slot/epoch for genesis - use init_ol_genesis instead"
    );
    (parent_state.cur_slot() + 1, parent_state.cur_epoch())
}

/// Constructs a block with per-transaction staging to filter invalid transactions.
///
/// Mimics STF's `construct_block` but with per-tx staging that:
/// 1. Fetches parent header
/// 2. Executes block initialization (epoch initial + block start)
/// 3. Validates each transaction against accumulated state
/// 4. Filters out invalid transactions (proof failures, execution failures)
/// 5. Fetches and admits buried ASM manifests for this block
/// 6. Applies the sealing policy
/// 7. Builds the complete block with only valid transactions
#[expect(clippy::too_many_arguments, reason = "can't get around the args")]
#[tracing::instrument(
    skip_all,
    fields(
        slot = block_slot,
        epoch = block_epoch,
        mempool_tx_count = mempool_txs.len(),
    ),
)]
pub(crate) async fn construct_block<C, E>(
    ctx: &C,
    epoch_sealing_policy: &E,
    config: &BlockGenerationConfig,
    parent_state: Arc<C::State>,
    block_slot: Slot,
    block_epoch: Epoch,
    mempool_txs: Vec<(OLTxId, OLTransaction)>,
    resource_state_before_block: EpochResourceState,
    bridge_params: BridgeParams,
) -> BlockAssemblyResult<ConstructBlockOutput<C::State>>
where
    C: BlockAssemblyAnchorContext + AccumulatorProofGenerator,
    E: EpochSealingPolicy,
    <<C::State as IStateAccessor>::AccountState as IAccountStateMut>::SnarkAccountStateMut: Clone,
{
    // Extract parent commitment from config.
    // Null parent means genesis - but genesis is built via `init_ol_genesis`, not block assembly.
    let parent_commitment = config.parent_block_commitment();
    assert!(
        !parent_commitment.is_null(),
        "construct_block called with null parent - genesis must be built via init_ol_genesis"
    );

    // Fetch parent block using BlockAssemblyAnchorContext trait
    let parent_blkid = *parent_commitment.blkid();
    let parent_block = ctx.fetch_ol_block(parent_blkid).await?.ok_or_else(|| {
        BlockAssemblyError::Db(DbError::Other(format!(
            "Parent block not found for blkid: {parent_blkid}"
        )))
    })?;

    // Create `BlockInfo` with placeholder timestamp (0) for STF processing.
    // Actual timestamp is computed at the end when building the header.
    let block_info = BlockInfo::new(0, block_slot, block_epoch);
    let block_context = BlockContext::new(&block_info, Some(parent_block.header()));

    // Create output buffer to collect logs from all transaction executions.
    let output_buffer = ExecOutputBuffer::new_empty();

    // Phase 1: Execute block initialization (epoch initial + block start).
    // Resource state flows through each phase, accumulating state diffs, logs, and manifest count.
    let epoch_cumulative_da = resource_state_before_block.da().clone();
    let epoch_cumulative_manifest_count = resource_state_before_block.manifest_count();
    let (accumulated_batch, accumulated_da) =
        execute_block_initialization(parent_state.as_ref(), &block_context, epoch_cumulative_da);

    // Phase 2: Process transactions, filtering out invalid ones.
    let ProcessTransactionsOutput {
        successful_txs,
        failed_txs,
        accumulated_batch,
        accumulated_da,
        mut sealing_limit_verdict,
    } = process_transactions(
        ctx,
        epoch_sealing_policy,
        &block_context,
        &output_buffer,
        parent_state.as_ref(),
        accumulated_batch,
        mempool_txs,
        accumulated_da,
        epoch_cumulative_manifest_count,
        bridge_params,
    );

    // Phase 3: Fetch buried ASM manifests for this block. Manifest selection may also request
    // that this block becomes terminal.
    let SelectedAsmManifests {
        manifests: selected_asm_manifests,
        sealing_limit_verdict: manifest_limit_verdict,
    } = fetch_asm_manifests_for_block(
        ctx,
        epoch_sealing_policy,
        parent_state.as_ref(),
        epoch_cumulative_manifest_count,
    )
    .await?;
    sealing_limit_verdict.merge(manifest_limit_verdict);
    let selected_asm_manifest_count = u32::try_from(selected_asm_manifests.len())
        .expect("selected ASM manifest count fits in u32");
    let manifest_container = if selected_asm_manifests.is_empty() {
        None
    } else {
        Some(OLAsmManifestContainer::new(selected_asm_manifests)?)
    };

    // Phase 4: Ask the sealing policy whether this block should be terminal.
    let sealing_decision =
        epoch_sealing_policy.should_seal_epoch(block_slot, &sealing_limit_verdict);
    let should_seal = sealing_decision.should_seal();
    debug!(
        %block_slot,
        ?sealing_limit_verdict,
        ?sealing_decision,
        should_seal,
        "epoch seal decision"
    );

    // Phase 5: Finalize block construction.
    // Clone output_buffer: the clone goes to build_block_template (which adds manifest logs
    // for the header), the original is consumed below to append this block's tx logs to DA.
    debug!(
        successful_tx_count = successful_txs.len(),
        failed_tx_count = failed_txs.len(),
        is_terminal = should_seal,
        has_manifests = manifest_container.is_some(),
        "block construction summary",
    );
    let (template, post_state) = build_block_template(
        config,
        &block_context,
        &parent_state,
        BuildBlockTemplateInput {
            accumulated_batch,
            output_buffer: output_buffer.clone(),
            successful_txs,
            is_terminal: should_seal,
            manifest_container,
        },
    )?;

    // Append this block's logs to accumulated DA and package the epoch resource state.
    let mut accumulated_da = accumulated_da;
    accumulated_da.append_logs(&output_buffer.into_logs());
    let resource_state = EpochResourceState::new(
        accumulated_da,
        epoch_cumulative_manifest_count
            .checked_add(selected_asm_manifest_count)
            .expect("epoch ASM manifest count fits in u32"),
    );

    Ok(ConstructBlockOutput {
        template,
        failed_txs,
        post_state,
        resource_state,
    })
}

/// Fetches ASM manifests for a block using [`BlockAssemblyAnchorContext`].
///
/// Each block may include a contiguous sequence of buried manifests after the
/// parent state's last accepted L1 height. The sequence is capped by the per-block
/// SSZ bound and by remaining pending ASM-log capacity.
///
/// `epoch_cumulative_manifest_count` is the number of ASM manifests already carried
/// by preceding blocks in the current epoch.
async fn fetch_asm_manifests_for_block<C, E, S>(
    ctx: &C,
    epoch_sealing_policy: &E,
    parent_state: &S,
    epoch_cumulative_manifest_count: u32,
) -> BlockAssemblyResult<SelectedAsmManifests>
where
    C: BlockAssemblyAnchorContext,
    E: EpochSealingPolicy,
    S: IStateAccessor,
{
    let last_l1_height = parent_state.last_l1_height();
    let start_height = last_l1_height
        .checked_add(1)
        .ok_or_else(|| BlockAssemblyError::Other("l1 height overflow".to_string()))?;

    // Fetch manifests using BlockAssemblyAnchorContext trait
    let fetched_asm_manifests = ctx
        .fetch_asm_manifests_from(start_height, MAX_SEALING_MANIFEST_COUNT as u32)
        .await?;

    let fetched_asm_manifest_count = fetched_asm_manifests.len();
    let fetched_asm_log_count: usize = fetched_asm_manifests.iter().map(|m| m.logs().len()).sum();

    let remaining_pending_asm_log_slots =
        (MAX_PENDING_ASM_LOGS as usize).saturating_sub(parent_state.pending_asm_logs_len());
    let SelectedAsmManifests {
        manifests: selected_asm_manifests,
        sealing_limit_verdict,
    } = select_asm_manifests(
        epoch_sealing_policy,
        fetched_asm_manifests,
        remaining_pending_asm_log_slots,
        epoch_cumulative_manifest_count,
    );

    let selected_asm_manifest_count = selected_asm_manifests.len();
    let selected_asm_log_count: usize = selected_asm_manifests.iter().map(|m| m.logs().len()).sum();
    debug!(
        start_height,
        fetched_asm_manifest_count,
        fetched_asm_log_count,
        selected_asm_manifest_count,
        selected_asm_log_count,
        ?sealing_limit_verdict,
        "fetched and selected asm manifests for block",
    );

    Ok(SelectedAsmManifests {
        manifests: selected_asm_manifests,
        sealing_limit_verdict,
    })
}

/// Selects ASM manifests for the candidate OL block.
///
/// Pending ASM-log capacity is a hard admission bound only: when the next
/// manifest would overflow the pending-log queue, selection stops and the block
/// does not seal for that reason. The manifest-count sealing rule can request
/// either including the boundary manifest and sealing or rejecting the boundary
/// manifest and sealing with the previously selected manifests.
///
/// `epoch_cumulative_manifest_count` is the number of ASM manifests already carried
/// by preceding blocks in the current epoch.
///
/// Selection does not budget terminal ASM-log drain output. The OL STF currently
/// applies buffered ASM-log drain as state effects only, without OL logs or
/// checkpoint DA output; if that invariant changes, selection must budget that
/// output before admitting ASM manifests.
fn select_asm_manifests<E: EpochSealingPolicy>(
    epoch_sealing_policy: &E,
    candidate_asm_manifests: Vec<AsmManifest>,
    remaining_pending_asm_log_slots: usize,
    epoch_cumulative_manifest_count: u32,
) -> SelectedAsmManifests {
    let mut selected_asm_manifests = Vec::new();
    let mut selected_asm_log_count = 0usize;
    let mut sealing_limit_verdict = EpochSealingLimitVerdict::within_limits();

    for candidate_asm_manifest in candidate_asm_manifests {
        let candidate_asm_log_count = candidate_asm_manifest.logs().len();
        if selected_asm_log_count.saturating_add(candidate_asm_log_count)
            > remaining_pending_asm_log_slots
        {
            // Pending ASM-log capacity is a hard admission bound, not a sealing trigger. Stop
            // manifest inclusion and leave the remaining manifests for a later block; the epoch
            // still seals only through cadence or another sealing limit.
            break;
        }

        let candidate_manifest_count = epoch_cumulative_manifest_count
            .saturating_add(
                u32::try_from(selected_asm_manifests.len())
                    .expect("selected ASM manifest count fits in u32"),
            )
            .saturating_add(1);
        // Candidate count includes the manifest currently being evaluated: it is the
        // epoch-cumulative manifest total if this candidate is admitted.
        let stats =
            EpochSealingResourceStats::new(0, LogMetrics::default(), candidate_manifest_count);
        let verdict = epoch_sealing_policy.check_limits(&stats);

        match verdict.most_restrictive_action() {
            EpochSealingLimitAction::RejectCandidate => {
                sealing_limit_verdict.merge(verdict);
                break;
            }
            EpochSealingLimitAction::SealAfterAdmit => {
                selected_asm_manifests.push(candidate_asm_manifest);
                sealing_limit_verdict.merge(verdict);
                break;
            }
            EpochSealingLimitAction::Continue => {
                selected_asm_log_count += candidate_asm_log_count;
                selected_asm_manifests.push(candidate_asm_manifest);
            }
        }
    }

    SelectedAsmManifests {
        manifests: selected_asm_manifests,
        sealing_limit_verdict,
    }
}

/// Executes block initialization (epoch initial + block start) on a fresh write batch.
///
/// Runs through `DaAccumulatingState` so that slot/epoch mutations are captured
/// in the DA accumulator. Returns the write batch and the updated DA data.
fn execute_block_initialization<S: BlockAssemblyStateAccess>(
    parent_state: &S,
    block_context: &BlockContext<'_>,
    accumulated_da: AccumulatedDaData,
) -> (WriteBatch<S::AccountState>, AccumulatedDaData)
where
    <<S as IStateAccessor>::AccountState as IAccountStateMut>::SnarkAccountStateMut: Clone,
{
    let (accumulator, logs) = accumulated_da.into_parts();
    let write_state = WriteTrackingState::new_empty(parent_state);
    let mut da_state = DaAccumulatingState::new_with_accumulator(write_state, accumulator);

    // Process block start for every block (sets cur_slot, etc.)
    // Per spec: process_slot_start runs before process_epoch_initial.
    process_block_start(&mut da_state, block_context)
        .expect("block start processing should not fail");

    // Process epoch initial if this is the first block of the epoch.
    if block_context.is_epoch_initial() {
        let init_ctx = block_context.get_epoch_initial_context();
        process_epoch_initial(&mut da_state, &init_ctx)
            .expect("epoch initial processing should not fail");
    }

    let (accumulator, write_state) = da_state.into_parts();
    (
        write_state.into_batch(),
        AccumulatedDaData::new(accumulator, logs),
    )
}

/// Processes transactions with per-tx staging, filtering out failed ones.
///
/// Consumes `accumulated_da` and returns it with the updated accumulator.
/// Logs from this block are NOT appended here — they go into `output_buffer`
/// and must be collected by the caller after manifest processing.
#[tracing::instrument(
    skip_all,
    fields(component = "ol_block_assembly", tx_count = mempool_txs.len())
)]
#[expect(clippy::too_many_arguments, reason = "all arguments are required")]
fn process_transactions<P, E, S>(
    proof_gen: &P,
    epoch_sealing_policy: &E,
    block_context: &BlockContext<'_>,
    output_buffer: &ExecOutputBuffer,
    parent_state: &S,
    accumulated_batch: WriteBatch<S::AccountState>,
    mempool_txs: Vec<(OLTxId, OLTransaction)>,
    accumulated_da: AccumulatedDaData,
    epoch_cumulative_manifest_count: u32,
    bridge_params: BridgeParams,
) -> ProcessTransactionsOutput<S>
where
    P: AccumulatorProofGenerator,
    E: EpochSealingPolicy,
    S: BlockAssemblyStateAccess,
    <<S as IStateAccessor>::AccountState as IAccountStateMut>::SnarkAccountStateMut: Clone,
{
    let mut successful_txs = Vec::new();
    let mut failed_txs = Vec::new();
    let mut sealing_limit_verdict = EpochSealingLimitVerdict::within_limits();

    // Track log metrics incrementally for checkpoint size estimation.
    let mut log_metrics = LogMetrics::from_logs(accumulated_da.logs());

    // Split out the accumulator for DaAccumulatingState; logs are preserved and
    // reassembled at the end.
    let (accumulator, epoch_logs) = accumulated_da.into_parts();

    // Create staging state once, reuse across transactions.
    // We work directly on this state and only clone for backup before each tx.
    // On success: backup is discarded. On failure: restore from backup.
    let mut staging_state = DaAccumulatingState::new_with_accumulator(
        WriteTrackingState::new(parent_state, accumulated_batch),
        accumulator,
    );

    for (txid, mempool_tx) in mempool_txs {
        let tx_target = mempool_tx.target();
        let tx_type = mempool_tx.type_id();
        let effect_transfer_count = mempool_tx.data().effects().transfers_iter().count();
        let effect_message_count = mempool_tx.data().effects().messages_iter().count();
        let total_sent_sats = mempool_tx
            .data()
            .effects()
            .get_total_value_sent()
            .map(|amount| amount.to_sat());
        let sau_summary = SauSummary::from_tx_payload(mempool_tx.payload());
        let sau_seq_no = sau_summary.as_ref().map(|s| s.seq_no);
        let sau_new_next_msg_idx = sau_summary.as_ref().map(|s| s.new_next_msg_idx);
        let sau_processed_message_count = sau_summary.as_ref().map(|s| s.processed_message_count);

        // Step 1: Validate and generate accumulator proofs, convert to OL transaction.
        // This only reads from state, so no rollback needed on failure.
        let tx = match add_accumulator_proofs(proof_gen, &staging_state, mempool_tx) {
            Ok(tx) => tx,
            Err(e) => {
                let reason = block_assembly_error_to_mempool_reason(&e);
                warn!(
                    ?txid,
                    %tx_type,
                    ?tx_target,
                    ?sau_seq_no,
                    ?sau_new_next_msg_idx,
                    ?sau_processed_message_count,
                    effect_transfer_count,
                    effect_message_count,
                    ?total_sent_sats,
                    ?reason,
                    error = %e,
                    "failed to validate/generate proofs for transaction"
                );
                #[cfg(test)]
                eprintln!("TX CONVERSION FAILED: {e:?}");
                failed_txs.push((txid, reason));
                continue;
            }
        };

        // Step 2: Clone batch and accumulator as backup before execution.
        let backup_batch = staging_state.inner().batch().clone();
        let backup_accumulator = staging_state.accumulator().clone();

        // Step 3: Create per-tx output buffer and execute transaction.
        // Logs are only merged into main buffer on success; on failure they're discarded.
        let tx_buffer = ExecOutputBuffer::new_empty();
        let basic_ctx = BasicExecContext::new(*block_context.block_info(), &tx_buffer)
            .with_bridge_params(bridge_params);
        let tx_ctx = TxExecContext::new(&basic_ctx, block_context.parent_header());

        debug!(%txid, kind = %tx.payload().type_id(), "processing transaction");
        match process_single_tx(&mut staging_state, &tx, &tx_ctx) {
            Ok(()) => {
                // Tx executed successfully. Before committing side effects, check
                // the estimated checkpoint size against component and envelope limits.
                let tx_logs = tx_buffer.into_logs();

                let mut tentative = log_metrics;
                tentative.add_logs(&tx_logs);
                let da_diff_size = staging_state.accumulator().estimated_encoded_size();
                let stats = EpochSealingResourceStats::new(
                    da_diff_size,
                    tentative,
                    epoch_cumulative_manifest_count,
                );
                let verdict = epoch_sealing_policy.check_limits(&stats);

                match verdict.checkpoint_size_action() {
                    EpochSealingLimitAction::RejectCandidate => {
                        debug!(
                            da_diff_size,
                            ?tentative,
                            "checkpoint size limit exceeded, dropping tx"
                        );
                        staging_state = DaAccumulatingState::new_with_accumulator(
                            WriteTrackingState::new(parent_state, backup_batch),
                            backup_accumulator,
                        );
                        sealing_limit_verdict = verdict;
                        break;
                    }
                    EpochSealingLimitAction::SealAfterAdmit => {
                        debug!(
                            da_diff_size,
                            ?tentative,
                            "checkpoint size approaching limit, sealing epoch"
                        );
                        if output_buffer.emit_logs(tx_logs).is_err() {
                            debug!(?txid, "block log cap exceeded, rolling back tx");
                            staging_state = DaAccumulatingState::new_with_accumulator(
                                WriteTrackingState::new(parent_state, backup_batch),
                                backup_accumulator,
                            );
                        } else {
                            successful_txs.push(tx);
                        }
                        sealing_limit_verdict = verdict;
                        break;
                    }
                    EpochSealingLimitAction::Continue => {
                        if output_buffer.emit_logs(tx_logs).is_err() {
                            debug!(?txid, "block log cap exceeded, rolling back tx");
                            staging_state = DaAccumulatingState::new_with_accumulator(
                                WriteTrackingState::new(parent_state, backup_batch),
                                backup_accumulator,
                            );
                            break;
                        }
                        log_metrics = tentative;
                        successful_txs.push(tx);
                    }
                }
            }
            Err(e) => {
                #[cfg(test)]
                eprintln!("TX EXECUTION FAILED: {e:?}");

                // Failure: discard tx_buffer (logs) and restore state from backup.
                let reason = stf_exec_error_to_mempool_reason(&e);
                warn!(
                    ?txid,
                    %tx_type,
                    ?tx_target,
                    ?sau_seq_no,
                    ?sau_new_next_msg_idx,
                    ?sau_processed_message_count,
                    effect_transfer_count,
                    effect_message_count,
                    ?total_sent_sats,
                    ?reason,
                    error = %e,
                    "transaction execution failed during staging"
                );

                staging_state = DaAccumulatingState::new_with_accumulator(
                    WriteTrackingState::new(parent_state, backup_batch),
                    backup_accumulator,
                );
                failed_txs.push((txid, reason));
            }
        }
    }

    // Reassemble AccumulatedDaData with updated accumulator; epoch_logs unchanged
    // (this block's logs stay in output_buffer for the caller to collect later).
    let (accumulator, inner_state) = staging_state.into_parts();
    let accumulated_da = AccumulatedDaData::new(accumulator, epoch_logs);
    let accumulated_batch = inner_state.into_batch();

    ProcessTransactionsOutput {
        successful_txs,
        failed_txs,
        accumulated_batch,
        accumulated_da,
        sealing_limit_verdict,
    }
}

/// Builds the final block template from accumulated state and transactions.
///
/// Returns `(template, final_state)` where `final_state` is the post-block state.
fn build_block_template<S>(
    config: &BlockGenerationConfig,
    block_context: &BlockContext<'_>,
    parent_state: &Arc<S>,
    input: BuildBlockTemplateInput<S::AccountState>,
) -> BlockAssemblyResult<(FullBlockTemplate, S)>
where
    S: BlockAssemblyStateAccess,
{
    let BuildBlockTemplateInput {
        accumulated_batch,
        output_buffer,
        successful_txs,
        is_terminal,
        manifest_container,
    } = input;

    // Clone parent state and apply accumulated batch to get state after transactions
    let mut final_state = parent_state.as_ref().clone();
    final_state.apply_write_batch(accumulated_batch)?;

    // Buffer any manifests carried by this block into intraepoch state.
    if let Some(mc) = &manifest_container {
        process_block_manifests(&mut final_state, mc.manifests()).map_err(|e| {
            error!(?e, "manifest buffering failed");
            BlockAssemblyError::BlockConstruction(e)
        })?;
    }

    // At the epoch terminal, drain the buffered ASM logs, reset intraepoch
    // state, and advance the epoch.
    if is_terminal {
        let basic_ctx = BasicExecContext::new(*block_context.block_info(), &output_buffer);
        process_epoch_terminal(&mut final_state, &basic_ctx).map_err(|e| {
            error!(?e, "epoch terminal processing failed");
            BlockAssemblyError::BlockConstruction(e)
        })?;
    }

    // The single final state root after all of the block's processing.
    let state_root = final_state.compute_state_root()?;

    // Defense-in-depth: per-tx and manifest emission paths enforce the cap at
    // emit time, and this preserves an explicit terminal assembly invariant
    // check before finalizing the template.
    output_buffer
        .verify_logs_within_block_limit()
        .map_err(BlockAssemblyError::BlockConstruction)?;

    // Extract logs for computing logs root
    let logs = output_buffer.into_logs();

    // Build exec outputs to get header state root
    let exec_outputs = BlockExecOutputs::new(state_root, logs);
    let logs_root = exec_outputs.compute_block_logs_root();
    let header_state_root = *exec_outputs.state_root();

    // Extract slot/epoch from block context
    let block_slot = block_context.slot();
    let block_epoch = block_context.epoch();
    let parent_blkid = block_context.compute_parent_blkid();

    // Build tx segment and body, attaching any manifests this block carries.
    let tx_segment = OLTxSegment::new(successful_txs)?;
    let mut body = OLBlockBody::new_common(tx_segment);
    if let Some(mc) = manifest_container {
        body.set_manifests(mc);
    }
    let body_root = body.compute_hash_commitment();

    // Terminality is the explicit assembly decision, independent of whether the
    // block happens to carry manifests.
    let mut flags = BlockFlags::zero();
    flags.set_is_terminal(is_terminal);

    // Use timestamp from config if provided, otherwise compute from system time.
    // OL block timestamps are expressed in milliseconds since Unix epoch.
    let timestamp = config.ts().unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_millis() as u64
    });

    // Build header
    let header = OLBlockHeader::new(
        timestamp,
        flags,
        block_slot,
        block_epoch,
        parent_blkid,
        body_root,
        header_state_root,
        logs_root,
    );

    // Build full block template
    let template = FullBlockTemplate::new(header, body);
    Ok((template, final_state))
}

/// Per-transaction Snark Account Update fields surfaced in tx-level
/// `warn!` events for debugging.
#[derive(Debug, Clone, Copy)]
struct SauSummary {
    seq_no: u64,
    new_next_msg_idx: u64,
    processed_message_count: usize,
}

impl SauSummary {
    /// Returns a [`SauSummary`] for [`TransactionPayload::SnarkAccountUpdate`]
    /// payloads, or `None` for payloads without an SAU update (e.g. plain
    /// account messages).
    fn from_tx_payload(payload: &TransactionPayload) -> Option<Self> {
        match payload {
            TransactionPayload::SnarkAccountUpdate(p) => {
                let operation = p.operation();
                Some(SauSummary {
                    seq_no: operation.update().seq_no(),
                    new_next_msg_idx: operation.update().proof_state().new_next_msg_idx(),
                    processed_message_count: operation.messages_iter().count(),
                })
            }
            TransactionPayload::GenericAccountMessage(_) => None,
        }
    }
}

/// Adds accumulator proofs for [`TransactionPayload::SnarkAccountUpdate`] transactions.
///
/// [`TransactionPayload::GenericAccountMessage`] transactions do not require
/// accumulator proofs and are returned unchanged.
///
/// Uses [`TxProofIndexer`] with [`verify_snark_acct_update_proofs`] to discover
/// what accumulator proofs are needed, then generates them via the
/// [`AccumulatorProofGenerator`].
fn add_accumulator_proofs<P: AccumulatorProofGenerator, S: IStateAccessor>(
    proof_gen: &P,
    state: &S,
    mempool_tx: OLTransaction,
) -> BlockAssemblyResult<OLTransaction> {
    let sau_payload = match mempool_tx.payload() {
        TransactionPayload::SnarkAccountUpdate(payload) => payload,
        TransactionPayload::GenericAccountMessage(_) => return Ok(mempool_tx),
    };
    let target = *sau_payload.target();
    let effects = mempool_tx.data().effects();

    // Use the TxProofIndexer to discover what proofs are needed by running the
    // verification logic in "dry-run" mode.
    let account_state = state
        .get_account_state(target)
        .map_err(BlockAssemblyError::State)?
        .ok_or(BlockAssemblyError::AccountNotFound(target))?;

    let mut proof_indexer = TxProofIndexer::new_fresh();
    verify_snark_acct_update_proofs(
        target,
        account_state,
        sau_payload.operation(),
        effects,
        &mut proof_indexer,
    )
    .map_err(BlockAssemblyError::SnarkUpdatePreValidation)?;

    // Generate accumulator proofs for the indexed claims, preserving order.
    let inbox_leaf_count = account_state
        .as_snark_account()
        .map_err(BlockAssemblyError::State)?
        .inbox_mmr()
        .num_entries();

    let mut all_acc_proofs = Vec::new();
    for check in proof_indexer.accumulator_checks() {
        match check {
            AccProofCheck::L1BlockRef(claim) => {
                let proofs =
                    proof_gen.generate_l1_block_ref_proofs(slice::from_ref(claim), state)?;
                all_acc_proofs.extend(proofs.l1_block_ref_proofs().iter().cloned());
            }
            AccProofCheck::Inbox(claim) => {
                let inbox_proofs = proof_gen.generate_inbox_proofs_for_claims(
                    target,
                    slice::from_ref(claim),
                    inbox_leaf_count,
                )?;
                all_acc_proofs.extend(inbox_proofs);
            }
        }
    }

    debug!(
        component = "ol_block_assembly",
        target = ?target,
        acc_proof_count = all_acc_proofs.len(),
        pred_check_count = proof_indexer.predicate_checks().len(),
        l1_block_refs_mmr_entries = state.l1_block_refs_mmr().num_entries(),
        "generated proofs for snark update via indexer"
    );

    let acc_proofs = RawMerkleProofList::from_vec_nonempty(all_acc_proofs);
    Ok(mempool_tx.with_accumulator_proofs(acc_proofs))
}

#[cfg(test)]
mod tests {
    const CHECKPOINT_MSG_VALUE_SATS: u64 = 100_000_000;

    use strata_acct_types::*;
    use strata_asm_manifest_types::AsmLogEntry;
    use strata_asm_proto_checkpoint_types::MAX_OL_LOGS_PER_CHECKPOINT;
    use strata_identifiers::{Buf32, L1BlockId, L1Height, OLBlockId};
    use strata_ol_chain_types_new::{MAX_LOGS_PER_BLOCK, MAX_SEALING_MANIFEST_COUNT, OLLog};
    use strata_ol_state_support_types::MemoryStateBaseLayer;

    use super::*;
    use crate::{FixedSlotSealing, LimitAwareSealing, test_utils::*};

    type OlWriteBatch = WriteBatch<<MemoryStateBaseLayer as IStateAccessor>::AccountState>;

    const SEALING_MANIFEST_CAP: L1Height = MAX_SEALING_MANIFEST_COUNT as L1Height;

    async fn build_process_tx_env(account_id: AccountId) -> TestEnv {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_account(TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE));
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        TestEnv::from_fixture(fixture, parent_commitment)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_block_ref_proof_gen_success() {
        let account_id = test_account_id(1);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000))
            .with_asm_manifests([1]);
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let l1_block_ref_claim = fixture
            .l1_block_ref(1)
            .expect("claim for L1 height 1 should exist");

        // Create tx with claims from the tracker using builder
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_block_ref_claims(vec![l1_block_ref_claim])
            .build();

        let ctx = create_test_context(fixture.storage().clone());

        // Convert transaction (generates accumulator proofs).
        let result = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        );

        assert!(
            result.is_ok(),
            "Proof generation should succeed, got error: {:?}",
            result.as_ref().err()
        );

        let tx = result.unwrap();
        match tx.payload() {
            TransactionPayload::SnarkAccountUpdate(sau) => {
                // Verify the payload was constructed with the correct target
                assert_eq!(sau.target(), &account_id);
            }
            _ => panic!("Expected SnarkAccountUpdate transaction"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_proof_gen_success() {
        let account_id = test_account_id(1);
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000).with_inbox(messages.clone()));
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");

        // Create tx using builder
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_processed_messages(messages.clone())
            .build();

        let ctx = create_test_context(fixture.storage().clone());
        let result = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        );

        assert!(
            result.is_ok(),
            "Proof generation should succeed, got error: {:?}",
            result.as_ref().err()
        );

        let tx = result.unwrap();
        match tx.payload() {
            TransactionPayload::SnarkAccountUpdate(sau) => {
                // Verify the payload was constructed with correct target and messages
                assert_eq!(sau.target(), &account_id);
                let msg_count = sau.operation().messages_iter().count();
                assert_eq!(msg_count, 2, "Should have 2 messages in operation data");
            }
            _ => panic!("Expected SnarkAccountUpdate transaction"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_add_accumulator_proofs_missing_target_account() {
        let fixture_builder = TestStorageFixtureBuilder::new();
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let missing_account = test_account_id(99);
        let mempool_tx = MempoolSnarkTxBuilder::new(missing_account)
            .with_seq_no(0)
            .build();

        let ctx = create_test_context(fixture.storage().clone());
        let err = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        )
        .expect_err("missing target account should fail");
        assert!(
            matches!(err, BlockAssemblyError::AccountNotFound(id) if id == missing_account),
            "expected AccountNotFound for missing account, got: {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_add_accumulator_proofs_gam_passthrough() {
        let fixture_builder = TestStorageFixtureBuilder::new();
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let target = test_account_id(77);
        let mempool_tx = MempoolGamTxBuilder::new(target)
            .with_data(vec![1, 2, 3])
            .build();
        let proofs_before = mempool_tx.proofs().clone();

        let ctx = create_test_context(fixture.storage().clone());
        let out_tx = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        )
        .expect("GAM tx should pass through unchanged");

        // For GAM payloads this path should not inject accumulator proofs.
        assert_eq!(
            out_tx.proofs(),
            &proofs_before,
            "GAM tx proofs should remain unchanged"
        );
        assert!(
            matches!(
                out_tx.payload(),
                TransactionPayload::GenericAccountMessage(_)
            ),
            "GAM tx payload should remain GenericAccountMessage"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_block_ref_claim_hash_mismatch() {
        let account_id = test_account_id(1);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000))
            .with_asm_manifests([1]);
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let seeded_claim = fixture
            .l1_block_ref(1)
            .expect("claim for L1 height 1 should exist");

        // Create claim with correct MMR index but wrong hash.
        let wrong_hash = test_hash(99);
        assert_ne!(
            wrong_hash,
            seeded_claim.entry_hash(),
            "test setup: wrong hash should differ from seeded claim hash"
        );
        let invalid_claims = vec![AccumulatorClaim::new(seeded_claim.idx(), wrong_hash)];

        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_block_ref_claims(invalid_claims)
            .build();
        let ctx = create_test_context(fixture.storage().clone());
        let result = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        );
        assert!(result.is_err(), "Should fail with hash mismatch");
        let err = result.unwrap_err();
        assert!(
            matches!(err, BlockAssemblyError::L1BlockRefHashMismatch { .. }),
            "Expected L1BlockRefHashMismatch, got: {:?}",
            err
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_block_ref_claim_missing_index() {
        // Seed one L1 header so we can reuse its hash with a missing index.
        let account_id = test_account_id(1);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000))
            .with_asm_manifests([1]);
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let seeded_claim = fixture
            .l1_block_ref(1)
            .expect("claim for L1 height 1 should exist");

        // Create claim with non-existent index.
        let nonexistent_index = 999u64;
        let invalid_claims = vec![AccumulatorClaim::new(
            nonexistent_index,
            seeded_claim.entry_hash(),
        )];

        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_block_ref_claims(invalid_claims)
            .build();
        let ctx = create_test_context(fixture.storage().clone());
        let result = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        );

        assert!(result.is_err(), "Should fail with nonexistent index");
        let err = result.unwrap_err();
        assert!(
            matches!(
                &err,
                BlockAssemblyError::Db(DbError::MmrIndexOutOfRange { .. })
                    | BlockAssemblyError::Db(DbError::MmrLeafNotFound(_))
            ),
            "Expected Db(MmrIndexOutOfRange|MmrLeafNotFound), got: {:?}",
            err
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_block_ref_claim_with_only_genesis_prefill() {
        // No real manifests are seeded, so the MMR contains only the genesis
        // sentinel at index 0. A claim quoting an arbitrary hash at that index
        // must be rejected as a hash mismatch.
        let arbitrary_hash = test_hash(42);
        let invalid_claims = vec![AccumulatorClaim::new(0, arbitrary_hash)];

        let account_id = test_account_id(1);
        let fixture_builder =
            TestStorageFixtureBuilder::new().with_account(TestAccount::new(account_id, 100_000));
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");

        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_block_ref_claims(invalid_claims)
            .build();

        let ctx = create_test_context(fixture.storage().clone());
        let result = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        );

        assert!(matches!(
            result,
            Err(BlockAssemblyError::L1BlockRefHashMismatch { idx: 0, .. })
        ));
    }

    #[test]
    fn test_error_mapping_to_mempool_reason() {
        // Verify InvalidAccumulatorClaim maps to Invalid
        let claim_err =
            BlockAssemblyError::InvalidAccumulatorClaim("test hash mismatch".to_string());
        let reason = block_assembly_error_to_mempool_reason(&claim_err);
        assert!(
            matches!(reason, MempoolTxInvalidReason::Invalid),
            "InvalidAccumulatorClaim should map to Invalid, got: {:?}",
            reason
        );

        // Verify Acct errors (from validate_message_index) map to Invalid
        let acct_err = BlockAssemblyError::Acct(AcctError::InvalidMsgIndex {
            account_id: test_account_id(1),
            expected: 5,
            got: 10,
        });
        let reason = block_assembly_error_to_mempool_reason(&acct_err);
        assert!(
            matches!(reason, MempoolTxInvalidReason::Invalid),
            "Acct errors should map to Invalid, got: {:?}",
            reason
        );

        // Verify non-MMR Db errors map to Failed (infrastructure error)
        let db_err = BlockAssemblyError::Db(DbError::Other("test error".to_string()));
        let reason = block_assembly_error_to_mempool_reason(&db_err);
        assert!(
            matches!(reason, MempoolTxInvalidReason::Failed),
            "Db errors should map to Failed, got: {:?}",
            reason
        );

        for (err, expected) in [
            (
                block_assembly_error_to_mempool_reason(&BlockAssemblyError::InvalidSignature(
                    OLBlockId::null(),
                )),
                MempoolTxInvalidReason::Failed,
            ),
            (
                block_assembly_error_to_mempool_reason(&BlockAssemblyError::TimestampTooEarly(123)),
                MempoolTxInvalidReason::Failed,
            ),
            (
                stf_exec_error_to_mempool_reason(&ExecError::SignatureInvalid("tx")),
                MempoolTxInvalidReason::Invalid,
            ),
            (
                stf_exec_error_to_mempool_reason(&ExecError::TransactionExpired(1, 2)),
                MempoolTxInvalidReason::Invalid,
            ),
            (
                stf_exec_error_to_mempool_reason(&ExecError::TransactionNotMature(1, 2)),
                MempoolTxInvalidReason::Failed,
            ),
        ] {
            assert_eq!(err, expected);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_claim_missing_index() {
        let account_id = test_account_id(1);
        let source_account = test_account_id(2);
        let messages = generate_message_entries(1, source_account);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000).with_inbox(messages.clone()));
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");

        // Create transaction claiming to process messages at indices [5, 6]
        // which don't exist (only index 0 exists)
        let fake_messages = generate_message_entries(2, source_account);
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_processed_messages(fake_messages)
            .with_new_msg_idx(7) // Claims next_inbox_msg_idx = 7 after processing
            .build();

        let ctx = create_test_context(fixture.storage().clone());
        let result = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        );

        assert!(
            result.is_err(),
            "Should fail when claiming inbox messages that don't exist"
        );
        let err = result.unwrap_err();
        // MmrIndexOutOfRange maps to Failed (not Invalid) because the same DB error
        // is also used for ASM manifests where it is transient.
        let reason = block_assembly_error_to_mempool_reason(&err);
        assert!(
            matches!(
                reason,
                MempoolTxInvalidReason::Invalid | MempoolTxInvalidReason::Failed
            ),
            "Expected Invalid or Failed mempool reason for missing inbox claims, got: {:?}",
            reason
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_claim_invalid_msg_idx() {
        let account_id = test_account_id(1);
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_account(TestAccount::new(account_id, 100_000).with_inbox(messages.clone()));
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");

        // Account has next_inbox_msg_idx = 0 on-chain
        // Create transaction claiming WRONG new next_inbox_msg_idx
        // Claims to process 2 messages but sets next_inbox_msg_idx = 10 (should be 2)
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_processed_messages(messages.clone())
            .with_new_msg_idx(10) // Wrong! Should be 2
            .build();

        let ctx = create_test_context(fixture.storage().clone());
        let result = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        );

        assert!(
            result.is_err(),
            "Should fail with invalid message index claim"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                BlockAssemblyError::Acct(AcctError::InvalidMsgIndex { .. })
                    | BlockAssemblyError::SnarkUpdatePreValidation(ExecError::Acct(
                        AcctError::InvalidMsgIndex { .. }
                    ))
                    | BlockAssemblyError::InboxEntryHashMismatch { .. }
                    | BlockAssemblyError::Db(DbError::MmrLeafNotFound(_))
                    | BlockAssemblyError::Db(DbError::MmrLeafNotFoundForAccount(_, _))
                    | BlockAssemblyError::Db(DbError::MmrIndexOutOfRange { .. })
            ),
            "Expected InvalidMsgIndex or MMR db error, got: {:?}",
            err
        );
    }

    /// Verifies that when both inbox proofs and L1 header proofs are present,
    /// the accumulator proofs are ordered: L1 headers first, then inbox proofs.
    /// This must match the verification order in snark-acct-sys verification.rs.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_proof_ordering_asm_manifests_before_inbox() {
        let account_id = test_account_id(1);
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_asm_manifests([1])
            .with_account(TestAccount::new(account_id, 100_000).with_inbox(messages.clone()));
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let state = fixture
            .storage()
            .ol_state()
            .get_toplevel_ol_state_async(parent_commitment)
            .await
            .expect("fetch stored state")
            .expect("stored state missing");
        let l1_block_ref_claims = vec![
            fixture
                .l1_block_ref(1)
                .expect("claim for L1 height 1 should exist"),
        ];

        // Create tx with BOTH L1 claims and inbox messages
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_block_ref_claims(l1_block_ref_claims.clone())
            .with_processed_messages(messages.clone())
            .build();

        let ctx = create_test_context(fixture.storage().clone());
        let tx = add_accumulator_proofs(
            &ctx,
            &MemoryStateBaseLayer::new(state.as_ref().clone()),
            mempool_tx,
        )
        .expect("proof generation should succeed");
        let tx_proofs = tx.proofs();

        // Verify accumulator proofs exist and have correct count
        let acc_proofs = tx_proofs
            .accumulator_proofs()
            .expect("should have accumulator proofs");
        let n_l1_block_refs = l1_block_ref_claims.len();
        let n_inbox = messages.len();
        assert_eq!(
            acc_proofs.proofs().len(),
            n_l1_block_refs + n_inbox,
            "Should have {n_l1_block_refs} L1 block refs + {n_inbox} inbox = {} total accumulator proofs",
            n_l1_block_refs + n_inbox
        );

        // Verify predicate satisfier exists
        assert!(
            tx_proofs.predicate_satisfiers().is_some(),
            "Should have predicate satisfiers"
        );
    }

    /// Verifies that `ProofSatisfierList::single` constructs a valid single-element
    /// list even when proof bytes are empty (e.g. from a NoopProver).
    #[test]
    fn test_proof_satisfier_list_accepts_empty_bytes() {
        use strata_ol_chain_types_new::ProofSatisfierList;

        // Empty bytes should still produce a valid single-element list
        let result = ProofSatisfierList::single(vec![]);
        assert!(
            result.is_some(),
            "Empty proof bytes should still produce a valid satisfier list"
        );
        let list = result.unwrap();
        assert_eq!(list.proofs().len(), 1);

        // Non-empty bytes should also work
        let result = ProofSatisfierList::single(vec![0u8]);
        assert!(
            result.is_some(),
            "Non-empty proof bytes should produce Some"
        );
        let list = result.unwrap();
        assert_eq!(list.proofs().len(), 1);
    }

    // Helper to validate block slot and epoch
    fn check_block_slot_epoch(
        block_template: &FullBlockTemplate,
        expected_slot: u64,
        expected_epoch: u32,
    ) {
        let header = block_template.header();
        assert_eq!(
            header.slot(),
            expected_slot,
            "Block should be at slot {}",
            expected_slot
        );
        assert_eq!(
            header.epoch(),
            expected_epoch,
            "Block should be in epoch {}",
            expected_epoch
        );
    }

    // Helper to validate ASM manifest contents independently of terminality.
    fn check_block_asm_manifests(
        block_template: &FullBlockTemplate,
        expected_heights: &[L1Height],
    ) {
        let body = block_template.body();
        let manifest_cont = body.manifests();
        if expected_heights.is_empty() {
            assert!(
                manifest_cont.is_none(),
                "Block should not carry an empty manifest container"
            );
            return;
        }

        assert!(
            manifest_cont.is_some(),
            "Block should contain ASM manifests {:?}",
            expected_heights
        );

        let manifests = manifest_cont.unwrap().manifests();
        assert_eq!(
            manifests.len(),
            expected_heights.len(),
            "Should have {} ASM manifests",
            expected_heights.len()
        );

        for (i, expected_height) in expected_heights.iter().enumerate() {
            assert_eq!(
                manifests[i].height(),
                *expected_height,
                "Manifest {} should have height {}",
                i,
                expected_height
            );
        }
    }

    // Helper to validate non-terminality independently of manifest presence.
    fn check_non_terminal_header(block_template: &FullBlockTemplate) {
        assert!(
            !block_template.header().is_terminal(),
            "Block should not be terminal"
        );
    }

    fn check_terminal_header(block_template: &FullBlockTemplate) {
        assert!(
            block_template.header().is_terminal(),
            "Block should be terminal"
        );
    }

    fn raw_asm_log(seed: u8) -> AsmLogEntry {
        AsmLogEntry::from_raw(vec![seed]).expect("raw test ASM log should fit")
    }

    // Helper to build blocks from start_commitment up to (but not including) target_slot.
    // Stores blocks and states so subsequent blocks can find their parent.
    async fn build_blocks_to_slot(env: &mut TestEnv, target_slot: u64) -> OLBlockCommitment {
        let mut current_commitment = env.parent_commitment();
        let mut resource_state = EpochResourceState::new_empty();

        let start_slot = if current_commitment.is_null() {
            0
        } else {
            env.parent_commitment().slot() + 1
        };

        for slot in start_slot..target_slot {
            let output = env
                .construct_empty_block_with_resource_state(resource_state)
                .await
                .unwrap_or_else(|e| panic!("Block construction at slot {slot} failed: {e:?}"));

            resource_state = if output.template.header().is_terminal() {
                EpochResourceState::new_empty()
            } else {
                output.resource_state.clone()
            };
            current_commitment = env.persist(&output).await;
        }

        current_commitment
    }

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic(expected = "generate_block_template_inner called with null parent")]
    async fn test_block_assembly_panics_on_null_parent() {
        let env_builder = TestStorageFixtureBuilder::new();
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let _ = env.generate_block_template().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_non_terminal_block_at_slot_1() {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let result = env.generate_block_template().await;
        assert!(
            result.is_ok(),
            "Block generation should succeed: {:?}",
            result.err()
        );

        let block_template = result.unwrap().into_template();
        check_block_slot_epoch(&block_template, 1, 1);
        check_non_terminal_header(&block_template);
        check_block_asm_manifests(&block_template, &[2, 3]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_block_template_fallback_timestamp_uses_milliseconds() {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let result = env
            .generate_block_template()
            .await
            .expect("block generation should succeed");

        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let timestamp = result.template().header().timestamp();

        assert!(
            (before..=after).contains(&timestamp),
            "fallback timestamp should use current time in milliseconds, got {timestamp} outside {before}..={after}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_terminal_block_at_slot_10() {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let mut env = TestEnv::from_fixture(fixture, parent_commitment);

        let _current_commitment = build_blocks_to_slot(&mut env, 10).await;

        let result = env.generate_block_template().await;
        assert!(
            result.is_ok(),
            "Block generation should succeed: {:?}",
            result.err()
        );

        let block_template = result.unwrap().into_template();
        check_block_slot_epoch(&block_template, 10, 1);
        check_terminal_header(&block_template);
        check_block_asm_manifests(&block_template, &[]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_non_terminal_block_below_max_sealing_manifests() {
        let manifest_count = SEALING_MANIFEST_CAP - 1;
        let env_builder = TestStorageFixtureBuilder::new()
            .with_genesis_parent_and_l1_manifest_count(manifest_count);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let parent_l1_height = env.parent_last_l1_height().await;
        let output = env
            .construct_empty_block()
            .await
            .expect("block generation should succeed");

        let expected_heights: Vec<L1Height> =
            ((parent_l1_height + 1)..=(parent_l1_height + manifest_count)).collect();
        check_non_terminal_header(&output.template);
        check_block_asm_manifests(&output.template, &expected_heights);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_manifest_budget_caps_and_seals_block() {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_genesis_parent_and_l1_manifest_count(SEALING_MANIFEST_CAP + 1);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let parent_l1_height = env.parent_last_l1_height().await;
        let output = env
            .construct_empty_block()
            .await
            .expect("block generation should succeed");

        let expected_heights: Vec<L1Height> =
            ((parent_l1_height + 1)..=(parent_l1_height + SEALING_MANIFEST_CAP)).collect();
        check_terminal_header(&output.template);
        check_block_asm_manifests(&output.template, &expected_heights);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_manifest_budget_seals_at_exact_max_boundary() {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_genesis_parent_and_l1_manifest_count(SEALING_MANIFEST_CAP);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let parent_l1_height = env.parent_last_l1_height().await;
        let output = env
            .construct_empty_block()
            .await
            .expect("block generation should succeed");

        let expected_heights: Vec<L1Height> =
            ((parent_l1_height + 1)..=(parent_l1_height + SEALING_MANIFEST_CAP)).collect();
        check_terminal_header(&output.template);
        check_block_asm_manifests(&output.template, &expected_heights);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_manifest_backlog_drains_across_blocks() {
        let rollover_count: L1Height = 2;
        let env_builder = TestStorageFixtureBuilder::new()
            .with_genesis_parent_and_l1_manifest_count(SEALING_MANIFEST_CAP + rollover_count);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let mut env = TestEnv::from_fixture(fixture, parent_commitment);

        let first_parent_l1_height = env.parent_last_l1_height().await;
        let first_output = env
            .construct_empty_block()
            .await
            .expect("first block generation should succeed");
        let first_expected_heights: Vec<L1Height> = ((first_parent_l1_height + 1)
            ..=(first_parent_l1_height + SEALING_MANIFEST_CAP))
            .collect();
        check_terminal_header(&first_output.template);
        check_block_asm_manifests(&first_output.template, &first_expected_heights);
        env.persist(&first_output).await;

        let second_parent_l1_height = env.parent_last_l1_height().await;
        let second_output = env
            .construct_empty_block()
            .await
            .expect("second block generation should succeed");
        let second_expected_heights: Vec<L1Height> =
            ((second_parent_l1_height + 1)..=(second_parent_l1_height + rollover_count)).collect();
        check_non_terminal_header(&second_output.template);
        check_block_asm_manifests(&second_output.template, &second_expected_heights);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_manifest_fetch_stops_at_cap() {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_genesis_parent_and_l1_manifest_count(SEALING_MANIFEST_CAP + 1);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);
        let parent_l1_height = env.parent_last_l1_height().await;
        let first_height_past_cap = parent_l1_height + SEALING_MANIFEST_CAP + 1;

        let missing_manifest_blkid = L1BlockId::from(Buf32::from([0xCD; 32]));
        env.storage()
            .l1()
            .revert_canonical_chain_async(first_height_past_cap - 1)
            .await
            .expect("revert L1 canonical chain to cap boundary");
        env.storage()
            .l1()
            .extend_canonical_chain_async(&missing_manifest_blkid, first_height_past_cap)
            .await
            .expect("insert missing-manifest canonical entry past cap");
        // ASM tip still points at the original height's blkid; the fetch path only reads its
        // height.

        let output = env
            .construct_empty_block()
            .await
            .expect("block generation should not read past cap");

        let expected_heights: Vec<L1Height> =
            ((parent_l1_height + 1)..=(parent_l1_height + SEALING_MANIFEST_CAP)).collect();
        check_block_asm_manifests(&output.template, &expected_heights);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_manifest_boundary_from_last_l1_height() {
        // Set last_l1_height to 2, but only provide manifests starting at 3.
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(1)
            .with_asm_manifests([1, 2])
            .with_l1_manifest_height_range(3..=4);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let result = env
            .generate_block_template()
            .await
            .expect("Block generation should succeed");

        let block_template = result.into_template();
        check_block_asm_manifests(&block_template, &[3, 4]);
    }

    #[test]
    fn test_pending_asm_log_capacity_limits_selected_manifests() {
        let policy = LimitAwareSealing::new(FixedSlotSealing::new(TEST_SLOTS_PER_EPOCH));
        let selected = select_asm_manifests(
            &policy,
            vec![
                create_l1_manifest_with_logs(2, vec![raw_asm_log(1)]),
                create_l1_manifest_with_logs(3, vec![raw_asm_log(2)]),
                create_l1_manifest_with_logs(4, vec![raw_asm_log(3)]),
            ],
            2,
            0,
        );

        assert_eq!(
            selected
                .manifests
                .iter()
                .map(|manifest| manifest.height())
                .collect::<Vec<_>>(),
            vec![2, 3],
            "selection should keep only the prefix that fits pending ASM-log capacity"
        );
        assert_eq!(
            selected.sealing_limit_verdict.most_restrictive_action(),
            EpochSealingLimitAction::Continue,
            "pending ASM-log capacity should not request epoch sealing"
        );
    }

    #[test]
    fn test_full_pending_asm_log_capacity_selects_no_manifests_without_sealing() {
        let policy = LimitAwareSealing::new(FixedSlotSealing::new(TEST_SLOTS_PER_EPOCH));
        let selected = select_asm_manifests(
            &policy,
            vec![create_l1_manifest_with_logs(2, vec![raw_asm_log(1)])],
            0,
            0,
        );

        assert!(
            selected.manifests.is_empty(),
            "selection should be empty when no pending ASM-log capacity remains"
        );
        assert_eq!(
            selected.sealing_limit_verdict.most_restrictive_action(),
            EpochSealingLimitAction::Continue,
            "pending ASM-log capacity should not request epoch sealing"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_terminal_block_omits_manifests_when_none_new() {
        // Parent already tracks manifests up to ASM tip height 3.
        // The terminal block should be valid without emitting an empty manifest container.
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(1)
            .with_l1_manifest_height_range(1..=3)
            .with_asm_manifests([1, 2, 3]);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let mut env = TestEnv::from_fixture(fixture, parent_commitment);

        let _current_commitment = build_blocks_to_slot(&mut env, 10).await;
        let output = env
            .generate_block_template()
            .await
            .expect("terminal block generation should succeed");

        let template = output.into_template();
        check_terminal_header(&template);
        check_block_asm_manifests(&template, &[]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_terminal_block_omits_manifests_when_start_above_asm_tip() {
        // Parent state claims a last_l1_height above ASM tip.
        // Fetch should return an empty manifest set (not an error).
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(1)
            .with_l1_manifest_height_range(1..=3)
            .with_asm_manifests([1, 2, 3, 4, 5]);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let mut env = TestEnv::from_fixture(fixture, parent_commitment);

        let _current_commitment = build_blocks_to_slot(&mut env, 10).await;
        let output = env
            .generate_block_template()
            .await
            .expect("terminal block generation should succeed");

        let template = output.into_template();
        check_terminal_header(&template);
        check_block_asm_manifests(&template, &[]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_missing_manifest_in_expected_range_errors() {
        // Parent last_l1_height = 1, ASM tip = 2, so terminal fetch expects manifest at height 2.
        // Corrupt L1 canonical chain so height 2 points to a blockid with no manifest body.
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(1)
            .with_l1_manifest_height_range(1..=2)
            .with_asm_manifests([1]);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let missing_manifest_blkid = L1BlockId::from(Buf32::from([0xAB; 32]));
        env.storage()
            .l1()
            .revert_canonical_chain_async(1)
            .await
            .expect("revert L1 canonical chain to height 1");
        env.storage()
            .l1()
            .extend_canonical_chain_async(&missing_manifest_blkid, 2)
            .await
            .expect("insert missing-manifest canonical entry at height 2");

        let err = env
            .generate_block_template()
            .await
            .expect_err("missing manifest in expected block range should error");

        assert!(
            matches!(err, BlockAssemblyError::Db(DbError::Other(_))),
            "expected Db(Other(_)) for missing manifest in expected range, got {err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_terminal_empty_block_omits_manifest_container() {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let mut env = TestEnv::from_fixture(fixture, parent_commitment);

        let _current_commitment = build_blocks_to_slot(&mut env, 10).await;
        let output = env
            .generate_block_template()
            .await
            .expect("terminal block generation should succeed");

        let template = output.into_template();
        let body = template.body();
        let tx_count = body.tx_segment().map(|seg| seg.txs().len()).unwrap_or(0);

        assert_eq!(tx_count, 0, "terminal empty block should have zero txs");
        assert!(
            body.manifests().is_none(),
            "terminal block should omit the manifest container when no new manifests are available"
        );
        assert!(
            template.header().is_terminal(),
            "terminal header flag must be set"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_non_terminal_block_at_slot_11() {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let mut env = TestEnv::from_fixture(fixture, parent_commitment);

        let _current_commitment = build_blocks_to_slot(&mut env, 11).await;

        let result = env.generate_block_template().await;
        assert!(
            result.is_ok(),
            "Block generation should succeed: {:?}",
            result.err()
        );

        let block_template = result.unwrap().into_template();
        check_block_slot_epoch(&block_template, 11, 2);
        check_non_terminal_header(&block_template);
        check_block_asm_manifests(&block_template, &[]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_non_terminal_empty_block_invariants() {
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(1)
            .with_l1_manifest_height_range(1..=3)
            .with_asm_manifests([1, 2, 3]);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let output = env
            .generate_block_template()
            .await
            .expect("non-terminal block generation should succeed");

        let template = output.into_template();
        let body = template.body();
        let tx_count = body.tx_segment().map(|seg| seg.txs().len()).unwrap_or(0);

        assert_eq!(tx_count, 0, "non-terminal empty block should have zero txs");
        assert!(
            body.manifests().is_none(),
            "non-terminal empty block should not include manifests when none are newly buried"
        );
        assert!(
            !template.header().is_terminal(),
            "non-terminal header flag must not be set"
        );
        // Empty log sets must commit to the canonical zero logs root.
        assert_eq!(
            *template.header().logs_root(),
            Buf32::zero(),
            "non-terminal empty block should have zero logs root"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_valid_tx_included_in_block() {
        // Setup env with snark account (seq_no=0 initially)
        let account_id = test_account_id(1);
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(
                TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE).with_inbox(messages.clone()),
            );
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        // Create tx and add to mock provider
        let valid_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_processed_messages(messages.clone())
            .build();
        let txid = valid_tx.compute_txid();

        let mempool = env.mempool();

        mempool.add_transaction(txid, valid_tx);

        let output = env
            .generate_block_template()
            .await
            .expect("Block generation should succeed");

        // Assert: tx included in block
        let included = included_txids(output.template());
        assert_eq!(included.len(), 1, "Block should contain 1 transaction");
        assert_eq!(
            included,
            vec![txid],
            "Included tx should be the submitted tx"
        );
    }

    #[test]
    fn test_block_template_result_into_parts() {
        let header = create_test_parent_header();
        let body =
            OLBlockBody::new_common(OLTxSegment::new(vec![]).expect("Failed to create tx segment"));
        let template = FullBlockTemplate::new(header, body);

        let failed_txs = vec![(
            OLTxId::from(Buf32::from([1u8; 32])),
            MempoolTxInvalidReason::Invalid,
        )];

        let result = BlockTemplateResult::new(
            template,
            failed_txs.clone(),
            EpochResourceState::new_empty(),
        );
        let (out_template, out_failed, _resource_state) = result.into_parts();

        assert_eq!(
            out_template.get_blockid(),
            out_template.header().compute_blkid()
        );
        assert_eq!(out_failed, failed_txs);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inbox_mmr_claims() {
        // Setup env with two snark accounts
        let account1 = test_account_id(1);
        let account2 = test_account_id(2);
        let source_account = test_account_id(3);
        let real_messages = generate_message_entries(2, source_account);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(
                TestAccount::new(account1, DEFAULT_ACCOUNT_BALANCE)
                    .with_inbox(real_messages.clone()),
            )
            .with_account(TestAccount::new(account2, DEFAULT_ACCOUNT_BALANCE))
            .with_expected_inbox_message_indices([(account1, vec![0, 1])]);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        assert_eq!(
            env.inbox_message_claims_for_account(account1).len(),
            real_messages.len(),
            "fixture should expose inbox message claims for seeded messages"
        );

        // Valid tx for account1: messages exist in MMR, proof generation succeeds
        let valid_tx = MempoolSnarkTxBuilder::new(account1)
            .with_seq_no(0)
            .with_processed_messages(real_messages.clone())
            .build();
        let valid_txid = valid_tx.compute_txid();

        // Invalid tx for account2: fake message NOT in MMR, proof generation fails
        let fake_message = generate_message_entries(1, test_account_id(3))
            .pop()
            .unwrap();
        let invalid_tx = MempoolSnarkTxBuilder::new(account2)
            .with_seq_no(0)
            .with_processed_messages(vec![fake_message])
            .build();
        let invalid_txid = invalid_tx.compute_txid();

        // Build block
        let mempool = env.mempool();
        mempool.add_transaction(valid_txid, valid_tx);
        mempool.add_transaction(invalid_txid, invalid_tx);

        let output = env
            .generate_block_template()
            .await
            .expect("Block generation should succeed");

        // Assert: block has 1 transaction (valid included, invalid rejected)
        assert_eq!(
            included_txids(output.template()),
            vec![valid_txid],
            "Block should contain only the valid tx"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_asm_manifest_mmr_claims() {
        // Setup env with two snark accounts and manifests in both state and storage MMRs
        let account1 = test_account_id(1);
        let account2 = test_account_id(2);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(1) // Start from slot 1 instead of genesis to avoid genesis manifest conflicts
            .with_account(TestAccount::new(account1, DEFAULT_ACCOUNT_BALANCE))
            .with_account(TestAccount::new(account2, DEFAULT_ACCOUNT_BALANCE))
            .with_asm_manifests([1, 2]);
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        // Valid tx for account1: L1 block ref claims exist in both MMRs for the requested L1
        // height.
        let valid_claims = vec![
            env.l1_block_ref(1)
                .expect("claim for L1 height 1 should exist"),
        ];
        let valid_tx = MempoolSnarkTxBuilder::new(account1)
            .with_seq_no(0)
            .with_l1_block_ref_claims(valid_claims)
            .build();
        let valid_txid = valid_tx.compute_txid();

        // Invalid tx for account2: non-existent MMR index (no corresponding MMR leaf)
        let fake_hash = test_hash(99);
        let max_seeded_idx = env
            .l1_block_refs()
            .iter()
            .map(|claim| claim.idx())
            .max()
            .expect("seeded claims");
        let missing_idx = max_seeded_idx + 100;
        let invalid_claims = vec![AccumulatorClaim::new(missing_idx, fake_hash)];
        let invalid_tx = MempoolSnarkTxBuilder::new(account2)
            .with_seq_no(0)
            .with_l1_block_ref_claims(invalid_claims)
            .build();
        let invalid_txid = invalid_tx.compute_txid();

        // Build block
        let mempool = env.mempool();
        mempool.add_transaction(valid_txid, valid_tx);
        mempool.add_transaction(invalid_txid, invalid_tx);

        let output = env
            .generate_block_template()
            .await
            .expect("Block generation should succeed");

        // Assert: block has 1 transaction (valid included, invalid rejected)
        assert_eq!(
            included_txids(output.template()),
            vec![valid_txid],
            "Block should contain only the valid tx"
        );
    }

    /// Tests that dependent transactions with sequential seq_no are both included.
    /// tx1: seq_no=0, tx2: seq_no=1
    /// Both should succeed because tx2 sees tx1's state changes (seq_no incremented to 1).
    #[tokio::test(flavor = "multi_thread")]
    async fn test_sequential_seq_no_both_succeed() {
        let account_id = test_account_id(1);
        let source_account = test_account_id(2);
        let messages = generate_message_entries(4, source_account);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(
                TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE).with_inbox(messages.clone()),
            );
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        // tx1: seq_no=0, processes messages[0..2]
        let tx1_messages = messages[0..2].to_vec();
        let tx1 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_processed_messages(tx1_messages)
            .build();
        let tx1_id = tx1.compute_txid();

        // tx2: seq_no=1, processes messages[2..4]
        let tx2_messages = messages[2..4].to_vec();
        let tx2 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(1)
            .with_processed_messages(tx2_messages)
            .with_new_msg_idx(4) // After processing tx1's 2 + tx2's 2 = 4
            .build();
        let tx2_id = tx2.compute_txid();

        // Build block
        let mempool = env.mempool();

        mempool.add_transaction(tx1_id, tx1);
        mempool.add_transaction(tx2_id, tx2);

        let output = env
            .generate_block_template()
            .await
            .expect("Block generation should succeed");

        // Assert: both txs included (tx2 succeeds because it sees tx1's seq_no increment)
        let included = included_txids(output.template());
        assert_eq!(
            included.len(),
            2,
            "Block should contain both txs (tx2 sees tx1's state changes)"
        );
    }

    /// Tests duplicate submission of the exact same tx (same txid, same seq_no).
    /// First execution succeeds, duplicate replay in the same block is rejected.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_duplicate_seq_no_same_tx_second_rejected() {
        let account_id = test_account_id(1);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE));
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .build();
        let txid = tx.compute_txid();

        let mempool = env.mempool();
        mempool.add_transaction(txid, tx.clone());
        mempool.add_transaction(txid, tx);

        let result = env
            .generate_block_template()
            .await
            .expect("block generation should succeed");

        let (template, failed_txs, _da) = result.into_parts();
        let txs = template.body().tx_segment().expect("tx segment").txs();
        assert_eq!(txs.len(), 1, "duplicate replay should not be included");
        assert_eq!(
            failed_txs.len(),
            1,
            "one duplicate should be reported failed"
        );
        assert_eq!(
            failed_txs[0].0, txid,
            "failed duplicate should reference replayed txid"
        );
    }

    /// Tests same-account duplicate seq_no across two different txs.
    /// First tx succeeds, second tx with same seq_no is rejected.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_duplicate_seq_no_different_tx_second_rejected() {
        let account_id = test_account_id(1);
        let receiver = test_account_id(2);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE))
            .with_account(TestAccount::new(receiver, 0));
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let tx1 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .build();
        let tx1_id = tx1.compute_txid();
        let tx2 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_outputs(vec![(receiver, 1)])
            .build();
        let tx2_id = tx2.compute_txid();

        let mempool = env.mempool();
        mempool.add_transaction(tx1_id, tx1);
        mempool.add_transaction(tx2_id, tx2);

        let result = env
            .generate_block_template()
            .await
            .expect("block generation should succeed");

        let (template, failed_txs, _da) = result.into_parts();
        let txs = template.body().tx_segment().expect("tx segment").txs();
        assert_eq!(txs.len(), 1, "only first seq_no=0 tx should be included");
        assert_eq!(txs[0].compute_txid(), tx1_id);
        assert_eq!(failed_txs.len(), 1, "second tx should be reported failed");
        assert_eq!(failed_txs[0].0, tx2_id);
    }

    /// Tests reverse ordering: seq_no=1 before seq_no=0.
    /// First tx fails, second tx succeeds against unchanged account state.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_reverse_order_seq_one_then_zero() {
        let account_id = test_account_id(1);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE));
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let tx_seq1 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(1)
            .build();
        let tx_seq1_id = tx_seq1.compute_txid();
        let tx_seq0 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .build();
        let tx_seq0_id = tx_seq0.compute_txid();

        let mempool = env.mempool();
        mempool.add_transaction(tx_seq1_id, tx_seq1);
        mempool.add_transaction(tx_seq0_id, tx_seq0);

        let result = env
            .generate_block_template()
            .await
            .expect("block generation should succeed");

        let (template, failed_txs, _da) = result.into_parts();
        let txs = template.body().tx_segment().expect("tx segment").txs();
        assert_eq!(txs.len(), 1, "only seq_no=0 tx should be included");
        assert_eq!(txs[0].compute_txid(), tx_seq0_id);
        assert_eq!(failed_txs.len(), 1, "seq_no=1 should be reported failed");
        assert_eq!(failed_txs[0].0, tx_seq1_id);
    }

    /// Tests seq gap behavior: seq_no=0 then seq_no=2 in the same block.
    /// The gap transaction must be rejected.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_seq_gap_zero_then_two_second_rejected() {
        let account_id = test_account_id(1);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE));
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let tx_seq0 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .build();
        let tx_seq0_id = tx_seq0.compute_txid();
        let tx_seq2 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(2)
            .build();
        let tx_seq2_id = tx_seq2.compute_txid();

        let mempool = env.mempool();
        mempool.add_transaction(tx_seq0_id, tx_seq0);
        mempool.add_transaction(tx_seq2_id, tx_seq2);

        let result = env
            .generate_block_template()
            .await
            .expect("block generation should succeed");

        let (template, failed_txs, _da) = result.into_parts();
        let txs = template.body().tx_segment().expect("tx segment").txs();
        assert_eq!(txs.len(), 1, "gap tx should not be included");
        assert_eq!(txs[0].compute_txid(), tx_seq0_id);
        assert_eq!(failed_txs.len(), 1, "gap tx should be reported failed");
        assert_eq!(failed_txs[0].0, tx_seq2_id);
    }

    /// Tests seq chain continuity across blocks when transactions are split one-per-block.
    /// This models `max_txs_per_block=1` execution behavior.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_seq_chain_across_blocks_when_split_one_per_block() {
        let account_id = test_account_id(1);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE));
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let mut env = TestEnv::from_fixture(fixture, parent_commitment);

        let tx_seq0 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .build();
        let tx_seq0_id = tx_seq0.compute_txid();
        let tx_seq1 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(1)
            .build();
        let tx_seq1_id = tx_seq1.compute_txid();

        // Build block 1 with seq_no=0.
        let output1 = env
            .construct_block(vec![(tx_seq0_id, tx_seq0)])
            .await
            .expect("block 1 should construct");
        let included1 = included_txids(&output1.template);
        assert_eq!(included1.len(), 1);
        let resource_state_after_block1 = output1.resource_state.clone();
        let _current_commitment = env.persist(&output1).await;

        // Build block 2 with seq_no=1 against the state produced by block 1.
        let output2 = env
            .construct_block_with_resource_state(
                vec![(tx_seq1_id, tx_seq1)],
                resource_state_after_block1,
            )
            .await
            .expect("block 2 should construct");
        let included2 = included_txids(&output2.template);
        assert_eq!(included2.len(), 1);
    }

    /// Tests that tx with seq_no=1 fails if tx with seq_no=0 is not present.
    /// Only tx2 (seq_no=1) submitted - should fail during execution because account has seq_no=0.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_dependent_tx_fails_without_predecessor() {
        let account_id = test_account_id(1);
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(
                TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE).with_inbox(messages.clone()),
            );
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        // Only submit tx with seq_no=1 (no seq_no=0 predecessor)
        // Block assembly will reject during execution because account has seq_no=0
        let tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(1)
            .with_processed_messages(messages.clone())
            .build();
        let txid = tx.compute_txid();

        let mempool = env.mempool();

        mempool.add_transaction(txid, tx);

        let output = env
            .generate_block_template()
            .await
            .expect("Block generation should succeed");

        // Block should have no txs - the seq_no=1 tx is rejected during execution
        let included = included_txids(output.template());
        assert_eq!(
            included.len(),
            0,
            "Block should be empty - tx with seq_no=1 rejected when account has seq_no=0"
        );
    }

    /// Tests that an independent tx succeeds even when another tx fails.
    /// tx1: account1 with invalid MMR claim (fails proof generation)
    /// tx2: account2 with valid empty tx (succeeds - different account, independent)
    #[tokio::test(flavor = "multi_thread")]
    async fn test_independent_tx_succeeds_when_other_fails() {
        let account1 = test_account_id(1);
        let account2 = test_account_id(2);
        let source_account = test_account_id(3);
        let account1_messages = generate_message_entries(2, source_account);
        let account2_messages = generate_message_entries(2, source_account);
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(
                TestAccount::new(account1, DEFAULT_ACCOUNT_BALANCE)
                    .with_inbox(account1_messages.clone()),
            )
            .with_account(
                TestAccount::new(account2, DEFAULT_ACCOUNT_BALANCE)
                    .with_inbox(account2_messages.clone()),
            );
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        // tx1: account1 claims fake message at index 100, but MMR only has indices 0-1
        let fake_message = generate_message_entries(1, test_account_id(99))
            .pop()
            .unwrap();
        let tx1 = MempoolSnarkTxBuilder::new(account1)
            .with_seq_no(0)
            .with_processed_messages(vec![fake_message])
            .with_new_msg_idx(100) // Claims invalid index
            .build();
        let tx1_id = tx1.compute_txid();

        // tx2: account2 with valid messages at correct indices
        let tx2 = MempoolSnarkTxBuilder::new(account2)
            .with_seq_no(0)
            .with_processed_messages(account2_messages.clone())
            .build();
        let tx2_id = tx2.compute_txid();

        // Build block
        let mempool = env.mempool();

        mempool.add_transaction(tx1_id, tx1);
        mempool.add_transaction(tx2_id, tx2);

        let output = env
            .generate_block_template()
            .await
            .expect("Block generation should succeed");

        // Assert: tx2 included, tx1 rejected
        assert_eq!(
            included_txids(output.template()),
            vec![tx2_id],
            "Block should contain tx2 only"
        );

        // Inner generation no longer reports invalid txs to mempool; both txs remain until
        // service-level reporting and block-application handling.
        let remaining = mempool.get_transactions(10).await.unwrap();
        assert_eq!(
            remaining.len(),
            2,
            "inner generation should not mutate mempool membership"
        );
    }

    /// Tests that tx1 sends balance to account2, and tx2 can spend that balance.
    /// tx1: account1 sends 1000 sats to account2
    /// tx2: account2 sends 500 sats (from received balance) to account3
    /// Both should succeed because tx2 sees tx1's balance transfer.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_balance_transfer_dependency_both_succeed() {
        let account1 = test_account_id(1);
        let account2 = test_account_id(2);
        let account3 = test_account_id(3);

        // Setup with custom balances: account1 has 10000 sats, account2 has 0 sats
        let env_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(TestAccount::new(account1, 10000))
            .with_account(TestAccount::new(account2, 0))
            .with_account(TestAccount::new(account3, 0));
        let (fixture, parent_commitment) = env_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        // tx1: account1 sends 1000 sats to account2
        let tx1 = MempoolSnarkTxBuilder::new(account1)
            .with_seq_no(0)
            .with_outputs(vec![(account2, 1000)])
            .build();
        let tx1_id = tx1.compute_txid();

        // tx2: account2 sends 500 sats to account3 (using balance received from tx1)
        let tx2 = MempoolSnarkTxBuilder::new(account2)
            .with_seq_no(0)
            .with_outputs(vec![(account3, 500)])
            .build();
        let tx2_id = tx2.compute_txid();

        // Build block
        let mempool = env.mempool();
        mempool.add_transaction(tx1_id, tx1);
        mempool.add_transaction(tx2_id, tx2);

        let output = env
            .generate_block_template()
            .await
            .expect("Block generation should succeed");

        // Assert: both txs included
        // tx1 executes first, transferring 1000 to account2
        // tx2 executes second, account2 now has 1000 and can send 500
        let included = included_txids(output.template());
        assert_eq!(
            included.len(),
            2,
            "Block should contain both txs (tx2 sees tx1's balance transfer)"
        );
    }

    /// Tests that a tx producing logs which would overflow the remaining block
    /// budget triggers a soft-break: the tx is not included, not marked invalid,
    /// and remains available for future blocks.
    ///
    /// Note: with `TxEffects::MAX_MESSAGES = 255` and `MAX_LOGS_PER_BLOCK = 4096`,
    /// a single tx can never overflow the per-tx log buffer on its own. So we
    /// pre-fill the block output buffer to near capacity and verify that even a
    /// small tx triggers the soft-break when it would push past the limit.
    async fn build_process_transactions_preamble(
        env: &TestEnv,
        timestamp: u64,
        slot_offset: u64,
    ) -> (
        Arc<MemoryStateBaseLayer>,
        OLBlockHeader,
        BlockInfo,
        OlWriteBatch,
        ExecOutputBuffer,
    ) {
        let parent_state = env
            .ctx()
            .fetch_state_for_tip(env.parent_commitment())
            .await
            .expect("fetch parent state")
            .expect("parent state exists");
        let parent_block = env
            .ctx()
            .fetch_ol_block(env.parent_commitment().blkid)
            .await
            .expect("fetch parent block")
            .expect("parent block exists");
        let parent_header = parent_block.header().clone();
        let block_info = BlockInfo::new(
            timestamp,
            parent_header.slot() + slot_offset,
            parent_header.epoch(),
        );
        let accumulated_batch = WriteBatch::default();
        let output_buffer = ExecOutputBuffer::new_empty();
        (
            parent_state,
            parent_header,
            block_info,
            accumulated_batch,
            output_buffer,
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_log_overflow_defers_tx() {
        let account_id = test_account_id(1);
        let env = build_process_tx_env(account_id).await;

        let (parent_state, parent_header, block_info, accumulated_batch, output_buffer) =
            build_process_transactions_preamble(&env, 1_000_001, 1).await;
        let block_context = BlockContext::new(&block_info, Some(&parent_header));

        // Create a tx with 5 withdrawal messages (= 5 logs).
        let withdrawal_dest = make_p2wpkh_bosd_descriptor(0x14);
        let tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_withdrawals(5, 100_000_000, withdrawal_dest)
            .build();
        let txid = tx.compute_txid();

        // Pre-fill output buffer so only 3 logs remain before the cap.
        // The tx produces 5 logs, so 5 > 3 remaining → soft-break.
        let prefill = MAX_LOGS_PER_BLOCK as usize - 3;
        output_buffer
            .emit_logs((0..prefill).map(|i| OLLog::new(AccountSerial::from(i as u32), vec![])))
            .expect("pre-fill should succeed");

        let out = process_transactions(
            env.ctx(),
            env.epoch_sealing_policy(),
            &block_context,
            &output_buffer,
            parent_state.as_ref(),
            accumulated_batch,
            vec![(txid, tx)],
            AccumulatedDaData::new_empty(),
            0,
            BridgeParams::default(),
        );

        assert!(
            out.successful_txs.is_empty(),
            "tx should not be included (logs would overflow)"
        );
        assert!(
            out.failed_txs.is_empty(),
            "soft-break should not mark tx invalid"
        );
    }

    /// Tests that when two txs are processed and the first fills the remaining
    /// log budget, the second is deferred via soft-break (not included, not
    /// marked invalid).
    ///
    /// Uses `process_transactions` directly with a pre-filled output buffer so
    /// that realistic message counts (≤ 255 per tx) can trigger the overflow.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_log_overflow_tx_skipped_not_invalid() {
        let account_id = test_account_id(8);
        let env = build_process_tx_env(account_id).await;

        let (parent_state, parent_header, block_info, accumulated_batch, output_buffer) =
            build_process_transactions_preamble(&env, 1_000_001, 1).await;
        let block_context = BlockContext::new(&block_info, Some(&parent_header));

        // First tx: 10 withdrawal messages = 10 logs.
        let withdrawal_dest = make_p2wpkh_bosd_descriptor(0x15);
        let tx_fill = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_withdrawals(10, 100_000_000, withdrawal_dest.clone())
            .build();
        let tx_fill_id = tx_fill.compute_txid();

        // Second tx: 1 withdrawal message = 1 log.
        let tx_overflow = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(1)
            .with_withdrawal(100_000_000, withdrawal_dest)
            .build();
        let tx_overflow_id = tx_overflow.compute_txid();

        // tx_fill emits 11 logs (10 withdrawals + 1 snark-update); tx_overflow emits 2.
        let prefill = MAX_LOGS_PER_BLOCK as usize - 11;
        output_buffer
            .emit_logs((0..prefill).map(|i| OLLog::new(AccountSerial::from(i as u32), vec![])))
            .expect("pre-fill should succeed");

        let out = process_transactions(
            env.ctx(),
            env.epoch_sealing_policy(),
            &block_context,
            &output_buffer,
            parent_state.as_ref(),
            accumulated_batch,
            vec![(tx_fill_id, tx_fill), (tx_overflow_id, tx_overflow)],
            AccumulatedDaData::new_empty(),
            0,
            BridgeParams::default(),
        );

        assert_eq!(
            out.successful_txs.len(),
            1,
            "only first tx should be included"
        );
        assert!(
            out.failed_txs.is_empty(),
            "block-full overflow tx should not be reported invalid"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_log_cap_soft_break() {
        let account_id = test_account_id(7);
        let env = build_process_tx_env(account_id).await;

        let (parent_state, parent_header, block_info, accumulated_batch, output_buffer) =
            build_process_transactions_preamble(&env, 1_000_001, 1).await;
        let block_context = BlockContext::new(&block_info, Some(&parent_header));

        let tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_withdrawal(100_000_000, make_p2wpkh_bosd_descriptor(0x16))
            .build();
        let txid = tx.compute_txid();

        // Pre-fill the block output buffer to exactly the cap so the next tx that emits logs
        // hits the soft-break branch without being marked invalid.
        output_buffer
            .emit_logs(
                (0..MAX_LOGS_PER_BLOCK).map(|i| OLLog::new(AccountSerial::from(i as u32), vec![])),
            )
            .expect("pre-filling up to the cap should succeed");

        let out = process_transactions(
            env.ctx(),
            env.epoch_sealing_policy(),
            &block_context,
            &output_buffer,
            parent_state.as_ref(),
            accumulated_batch,
            vec![(txid, tx)],
            AccumulatedDaData::new_empty(),
            0,
            BridgeParams::default(),
        );

        assert!(
            out.failed_txs.is_empty(),
            "soft-break overflow should not mark tx invalid"
        );
        assert!(
            out.successful_txs.is_empty(),
            "overflowing tx should not be included"
        );
    }

    async fn run_process_transactions_with_seeded_checkpoint_logs(
        account_id: AccountId,
        seeded_log_count: usize,
        mempool_txs: Vec<(OLTxId, OLTransaction)>,
    ) -> ProcessTransactionsOutput<MemoryStateBaseLayer> {
        const CHECKPOINT_TEST_TIMESTAMP: u64 = 1_000_003;
        const CHECKPOINT_TEST_SLOT_OFFSET: u64 = 3;

        let env = build_process_tx_env(account_id).await;
        let (parent_state, parent_header, block_info, accumulated_batch, output_buffer) =
            build_process_transactions_preamble(
                &env,
                CHECKPOINT_TEST_TIMESTAMP,
                CHECKPOINT_TEST_SLOT_OFFSET,
            )
            .await;
        let block_context = BlockContext::new(&block_info, Some(&parent_header));
        let seeded_da = seeded_da(seeded_log_count);

        process_transactions(
            env.ctx(),
            env.epoch_sealing_policy(),
            &block_context,
            &output_buffer,
            parent_state.as_ref(),
            accumulated_batch,
            mempool_txs,
            seeded_da,
            0,
            BridgeParams::default(),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_checkpoint_soft_commits_then_stops() {
        let account_id = test_account_id(9);
        let withdrawal_dest = make_p2wpkh_bosd_descriptor(0x17);
        let tx1 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_withdrawal(CHECKPOINT_MSG_VALUE_SATS, withdrawal_dest.clone())
            .build();
        let tx2 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(1)
            .with_withdrawal(CHECKPOINT_MSG_VALUE_SATS, withdrawal_dest)
            .build();
        let tx1_id = tx1.compute_txid();
        let tx2_id = tx2.compute_txid();

        // Seed one below soft threshold so tx1's single log tips verdict to
        // SoftLimitReached (commit current tx, then stop).
        let soft_threshold = MAX_OL_LOGS_PER_CHECKPOINT as usize * 9 / 10;
        let out = run_process_transactions_with_seeded_checkpoint_logs(
            account_id,
            soft_threshold - 1,
            vec![(tx1_id, tx1), (tx2_id, tx2)],
        )
        .await;

        assert_eq!(
            out.successful_txs.len(),
            1,
            "soft limit should commit current tx and defer remaining txs"
        );
        assert_eq!(
            out.successful_txs[0].compute_txid(),
            tx1_id,
            "first tx should be committed before soft-break"
        );
        assert!(
            out.failed_txs.is_empty(),
            "deferred tx should not be marked invalid"
        );
        assert!(
            matches!(
                out.sealing_limit_verdict.checkpoint_size_action(),
                EpochSealingLimitAction::SealAfterAdmit
            ),
            "soft verdict should request checkpoint-size sealing"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_checkpoint_hard_rolls_back_then_stops() {
        let account_id = test_account_id(10);
        let withdrawal_dest = make_p2wpkh_bosd_descriptor(0x18);
        let tx1 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_withdrawal(CHECKPOINT_MSG_VALUE_SATS, withdrawal_dest.clone())
            .build();
        let tx2 = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(1)
            .with_withdrawal(CHECKPOINT_MSG_VALUE_SATS, withdrawal_dest)
            .build();
        let tx1_id = tx1.compute_txid();
        let tx2_id = tx2.compute_txid();

        // tx1 emits 2 logs (withdrawal + snark-update).
        let control = run_process_transactions_with_seeded_checkpoint_logs(
            account_id,
            (MAX_OL_LOGS_PER_CHECKPOINT as usize) - 3,
            vec![(tx1_id, tx1.clone())],
        )
        .await;
        assert_eq!(
            control
                .successful_txs
                .iter()
                .map(OLTransaction::compute_txid)
                .collect::<Vec<_>>(),
            vec![tx1_id],
            "tx1 should be accepted just below the hard threshold"
        );

        let out = run_process_transactions_with_seeded_checkpoint_logs(
            account_id,
            (MAX_OL_LOGS_PER_CHECKPOINT as usize) - 2,
            vec![(tx1_id, tx1), (tx2_id, tx2)],
        )
        .await;

        assert!(
            out.successful_txs.is_empty(),
            "hard limit should roll back current tx"
        );
        assert!(
            out.failed_txs.is_empty(),
            "hard-limit rollback should not mark tx invalid"
        );
        assert!(
            matches!(
                out.sealing_limit_verdict.checkpoint_size_action(),
                EpochSealingLimitAction::RejectCandidate
            ),
            "hard verdict should request checkpoint-size sealing"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stress_1000_txs_100_accounts() {
        const ACCOUNT_COUNT: usize = 100;
        const TXS_PER_ACCOUNT: usize = 10;
        const INITIAL_BALANCE: u64 = 1_000_000;

        let account_ids: Vec<_> = (1..=ACCOUNT_COUNT)
            .map(|i| test_account_id(i as u8))
            .collect();

        let fixture_builder = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_accounts(
                account_ids
                    .iter()
                    .copied()
                    .map(|account_id| TestAccount::new(account_id, INITIAL_BALANCE)),
            );
        let (fixture, parent_commitment) = fixture_builder.build_fixture().await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let mut txs = Vec::with_capacity(ACCOUNT_COUNT * TXS_PER_ACCOUNT);
        for (idx, sender) in account_ids.iter().enumerate() {
            let receiver = account_ids[(idx + 1) % ACCOUNT_COUNT];
            for seq_no in 0..TXS_PER_ACCOUNT {
                let tx = MempoolSnarkTxBuilder::new(*sender)
                    .with_seq_no(seq_no as u64)
                    .with_outputs(vec![(receiver, 1)])
                    .build();
                let txid = tx.compute_txid();
                txs.push((txid, tx));
            }
        }

        let output = env
            .construct_block(txs)
            .await
            .expect("high-volume block should assemble");

        let included = included_txids(&output.template);
        assert_eq!(
            included.len(),
            ACCOUNT_COUNT * TXS_PER_ACCOUNT,
            "all high-volume txs should be included"
        );
        assert!(
            output.failed_txs.is_empty(),
            "high-volume transfer set should not produce failed txs"
        );

        for account_id in &account_ids {
            assert_eq!(
                account_balance(&output.post_state, *account_id),
                BitcoinAmount::from_sat(INITIAL_BALANCE),
                "cyclic transfers should preserve per-account net balance for every account"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stress_50_l1_claims_with_duplicates() {
        let account_id = test_account_id(50);
        let (fixture, parent_commitment) = TestStorageFixtureBuilder::new()
            .with_parent_slot(1)
            .with_account(TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE))
            .with_asm_manifests(1..=25)
            .build_fixture()
            .await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let mut claims: Vec<_> = env.l1_block_refs().to_vec();
        claims.extend(claims.clone());
        assert_eq!(claims.len(), 50, "stress setup should build 50 claims");

        let tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_l1_block_ref_claims(claims)
            .build();
        let txid = tx.compute_txid();

        let output = env
            .construct_block(vec![(txid, tx)])
            .await
            .expect("large-claim tx should assemble");

        let included = included_txids(&output.template);
        assert_eq!(included.len(), 1, "claim-heavy tx should be included");
        let included_tx = &output
            .template
            .body()
            .tx_segment()
            .expect("tx segment")
            .txs()[0];
        let acc_proofs = included_tx
            .proofs()
            .accumulator_proofs()
            .expect("claim-heavy tx should carry accumulator proofs");
        assert_eq!(
            acc_proofs.proofs().len(),
            50,
            "duplicate claims should currently produce duplicate accumulator proofs (no dedup)"
        );
        assert!(
            output.failed_txs.is_empty(),
            "claim-heavy tx should not be reported failed"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stress_100_inbox_messages_one_tx() {
        // Stress value intentionally below SAU_MAX_PROCESSED_MESSAGES (1<<16),
        // while still large enough to exercise message-heavy assembly/proof paths.
        const MSG_COUNT: usize = 100;

        let account_id = test_account_id(60);
        let source_account = test_account_id(61);
        let messages = generate_message_entries(MSG_COUNT, source_account);
        let (fixture, parent_commitment) = TestStorageFixtureBuilder::new()
            .with_parent_slot(0)
            .with_l1_manifest_height_range(1..=3)
            .with_account(
                TestAccount::new(account_id, DEFAULT_ACCOUNT_BALANCE).with_inbox(messages.clone()),
            )
            .build_fixture()
            .await;
        let env = TestEnv::from_fixture(fixture, parent_commitment);

        let tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_processed_messages(messages)
            .build();
        let txid = tx.compute_txid();

        let output = env
            .construct_block(vec![(txid, tx)])
            .await
            .expect("message-heavy tx should assemble");

        let included = included_txids(&output.template);
        assert_eq!(included.len(), 1, "message-heavy tx should be included");
        assert!(
            output.failed_txs.is_empty(),
            "message-heavy tx should not be reported failed"
        );
        assert_eq!(
            snark_account_next_inbox_msg_idx(&output.post_state, account_id),
            MSG_COUNT as u64,
            "processing should advance next_inbox_msg_idx by all messages"
        );
        assert_eq!(
            snark_account_seqno(&output.post_state, account_id),
            1,
            "account seqno should advance after processing update"
        );
    }
}
