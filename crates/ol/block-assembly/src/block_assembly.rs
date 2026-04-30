//! Block assembly logic.

use std::{
    slice,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use strata_config::SequencerConfig;
use strata_db_types::errors::DbError;
use strata_identifiers::{Epoch, OLBlockCommitment, OLTxId, Slot};
use strata_ledger_types::{
    AccProofCheck, IAccountState, ISnarkAccountState, IStateAccessor, TxProofIndexer, *,
};
use strata_ol_chain_types_new::*;
use strata_ol_mempool::MempoolTxInvalidReason;
use strata_ol_state_support_types::{DaAccumulatingState, WriteTrackingState};
use strata_ol_state_types::WriteBatch;
use strata_ol_stf::*;
use strata_snark_acct_types as _;
use tracing::{debug, error, warn};

use crate::{
    AccumulatorProofGenerator, BlockAssemblyResult, BlockAssemblyStateAccess, EpochSealingPolicy,
    MempoolProvider,
    checkpoint_size::{CheckpointSizeVerdict, LogMetrics, checkpoint_size_verdict},
    context::BlockAssemblyAnchorContext,
    da_tracker::AccumulatedDaData,
    error::BlockAssemblyError,
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
    /// Whether the estimated checkpoint payload is approaching the L1 envelope limit.
    checkpoint_size_limit_reached: bool,
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
        | ExecError::InsufficientAccountBalance(_, _) => MempoolTxInvalidReason::Failed,

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
        | BlockAssemblyError::L1HeaderHashMismatch { .. }
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
        | BlockAssemblyError::NoPendingTemplateForParent(_)
        | BlockAssemblyError::Other(_)
        | BlockAssemblyError::RequestChannelClosed
        | BlockAssemblyError::ResponseChannelClosed
        | BlockAssemblyError::UnknownTemplateId(_)
        | BlockAssemblyError::TimestampTooEarly(_)
        | BlockAssemblyError::BlockNotFound(_)
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
    #[cfg_attr(not(test), expect(dead_code, reason = "only used by tests"))]
    pub(crate) post_state: S,
    /// Accumulated DA data for the constructed block.
    pub(crate) accumulated_da: AccumulatedDaData,
}

/// Generate a block template from the given configuration.
///
/// Fetches transactions from the mempool, generates accumulator proofs, validates execution
/// with per-transaction staging, and constructs a complete block template.
///
/// Transactions that fail proof generation or execution are reported to the mempool.
///
/// Returns a [`BlockTemplateResult`] containing both the generated template and
/// any transactions that failed validation during assembly.
pub(crate) async fn generate_block_template_inner<C, E>(
    ctx: &C,
    epoch_sealing_policy: &E,
    sequencer_config: &SequencerConfig,
    block_generation_config: BlockGenerationConfig,
    parent_da: AccumulatedDaData,
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
        .ok_or_else(|| {
            BlockAssemblyError::Db(DbError::Other(format!(
                "Parent state not found for commitment: {parent_commitment}"
            )))
        })?;

    // 2. Calculate next slot and epoch
    let (block_slot, block_epoch) =
        calculate_block_slot_and_epoch(&parent_commitment, parent_state.as_ref());

    // 3. Get transactions from mempool
    let mempool_txs = MempoolProvider::get_transactions(ctx, max_txs_per_block).await?;

    // 4. Construct block (handles terminal detection and manifest fetching internally)
    let output = construct_block(
        ctx,
        epoch_sealing_policy,
        &block_generation_config,
        parent_state,
        block_slot,
        block_epoch,
        mempool_txs,
        parent_da,
    )
    .await?;

    // 5. Report failed transactions to mempool
    if !output.failed_txs.is_empty() {
        debug!(
            component = "ol_block_assembly",
            count = output.failed_txs.len(),
            "Reporting failed transactions to mempool"
        );
        MempoolProvider::report_invalid_transactions(ctx, &output.failed_txs).await?;
    }

    Ok(BlockTemplateResult::new(
        output.template,
        output.failed_txs,
        output.accumulated_da,
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
/// 5. Detects terminal blocks and fetches L1 manifests
/// 6. Builds the complete block with only valid transactions
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
    parent_da: AccumulatedDaData,
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
    // `AccumulatedDaData` flows through each phase, accumulating state diffs and logs.
    let (accumulated_batch, accumulated_da) =
        execute_block_initialization(parent_state.as_ref(), &block_context, parent_da);

    // Phase 2: Process transactions, filtering out invalid ones.
    let ProcessTransactionsOutput {
        successful_txs,
        failed_txs,
        accumulated_batch,
        accumulated_da,
        checkpoint_size_limit_reached,
    } = process_transactions(
        ctx,
        &block_context,
        &output_buffer,
        parent_state.as_ref(),
        accumulated_batch,
        mempool_txs,
        accumulated_da,
    );

    // Phase 3: Seal the epoch if the policy says so or the checkpoint payload is near the
    // L1 envelope limit. Fetch manifests for the terminal block (possibly empty if L1 is slow).
    let should_seal =
        epoch_sealing_policy.should_seal_epoch(block_slot) || checkpoint_size_limit_reached;
    debug!(%block_slot, checkpoint_size_limit_reached, should_seal, "epoch seal decision");
    let manifest_container = if should_seal {
        fetch_asm_manifests_for_terminal_block(ctx, parent_state.as_ref()).await?
    } else {
        None
    };

    // Phase 4: Finalize block construction.
    // Clone output_buffer: the clone goes to build_block_template (which adds manifest logs
    // for the header), the original is consumed below to append this block's tx logs to DA.
    debug!(
        successful_tx_count = successful_txs.len(),
        failed_tx_count = failed_txs.len(),
        is_terminal = manifest_container.is_some(),
        "block construction summary",
    );
    let (template, post_state) = build_block_template(
        config,
        &block_context,
        &parent_state,
        accumulated_batch,
        output_buffer.clone(),
        successful_txs,
        manifest_container,
    )?;

    // Append this block's logs to accumulated DA.
    let mut accumulated_da = accumulated_da;
    accumulated_da.append_logs(&output_buffer.into_logs());

    Ok(ConstructBlockOutput {
        template,
        failed_txs,
        post_state,
        accumulated_da,
    })
}

/// Fetches ASM manifests for a terminal block using `BlockAssemblyAnchorContext`.
///
/// Terminal blocks need to include all L1 blocks processed since the last terminal block.
/// This function fetches manifests from `parent_state.last_l1_height() + 1` up to the latest
/// available L1 block.
async fn fetch_asm_manifests_for_terminal_block<
    C: BlockAssemblyAnchorContext,
    S: IStateAccessor,
>(
    ctx: &C,
    parent_state: &S,
) -> BlockAssemblyResult<Option<OLL1ManifestContainer>> {
    let last_l1_height = parent_state.last_l1_height();
    let start_height = last_l1_height + 1;

    // Fetch manifests using BlockAssemblyAnchorContext trait
    let manifests = ctx.fetch_asm_manifests_from(start_height).await?;

    let total_logs: usize = manifests.iter().map(|m| m.logs().len()).sum();
    debug!(
        start_height,
        manifest_count = manifests.len(),
        total_logs,
        "fetched asm manifests for terminal block",
    );

    let container = OLL1ManifestContainer::new(manifests)?;

    // Return the container regardless of whether manifests is empty or not. Because otherwise, if
    // for some reasons L1 is slow, epoch sealing policy is not respected.
    Ok(Some(container))
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
    let accumulated_batch = WriteBatch::new_from_state(parent_state);
    let write_state = WriteTrackingState::new(parent_state, accumulated_batch);
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
fn process_transactions<P, S>(
    proof_gen: &P,
    block_context: &BlockContext<'_>,
    output_buffer: &ExecOutputBuffer,
    parent_state: &S,
    accumulated_batch: WriteBatch<S::AccountState>,
    mempool_txs: Vec<(OLTxId, OLTransaction)>,
    accumulated_da: AccumulatedDaData,
) -> ProcessTransactionsOutput<S>
where
    P: AccumulatorProofGenerator,
    S: BlockAssemblyStateAccess,
    <<S as IStateAccessor>::AccountState as IAccountStateMut>::SnarkAccountStateMut: Clone,
{
    let mut successful_txs = Vec::new();
    let mut failed_txs = Vec::new();
    let mut checkpoint_size_limit_reached = false;

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
        // Step 1: Validate and generate accumulator proofs, convert to OL transaction.
        // This only reads from state, so no rollback needed on failure.
        let tx = match add_accumulator_proofs(proof_gen, &staging_state, mempool_tx) {
            Ok(tx) => tx,
            Err(e) => {
                debug!(?txid, %e, "failed to validate/generate proofs for transaction");
                #[cfg(test)]
                eprintln!("TX CONVERSION FAILED: {e:?}");
                failed_txs.push((txid, block_assembly_error_to_mempool_reason(&e)));
                continue;
            }
        };

        // Step 2: Clone batch and accumulator as backup before execution.
        let backup_batch = staging_state.inner().batch().clone();
        let backup_accumulator = staging_state.accumulator().clone();

        // Step 3: Create per-tx output buffer and execute transaction.
        // Logs are only merged into main buffer on success; on failure they're discarded.
        let tx_buffer = ExecOutputBuffer::new_empty();
        let basic_ctx = BasicExecContext::new(*block_context.block_info(), &tx_buffer);
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
                let verdict = checkpoint_size_verdict(da_diff_size, &tentative);

                match verdict {
                    CheckpointSizeVerdict::HardLimitExceeded => {
                        debug!(
                            da_diff_size,
                            ?tentative,
                            "checkpoint size limit exceeded, dropping tx"
                        );
                        staging_state = DaAccumulatingState::new_with_accumulator(
                            WriteTrackingState::new(parent_state, backup_batch),
                            backup_accumulator,
                        );
                        checkpoint_size_limit_reached = true;
                        break;
                    }
                    CheckpointSizeVerdict::SoftLimitReached => {
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
                        checkpoint_size_limit_reached = true;
                        break;
                    }
                    CheckpointSizeVerdict::WithinLimits => {
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
                debug!(?txid, %e, "transaction execution failed during staging");

                staging_state = DaAccumulatingState::new_with_accumulator(
                    WriteTrackingState::new(parent_state, backup_batch),
                    backup_accumulator,
                );
                failed_txs.push((txid, stf_exec_error_to_mempool_reason(&e)));
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
        checkpoint_size_limit_reached,
    }
}

/// Builds the final block template from accumulated state and transactions.
///
/// Returns `(template, final_state)` where `final_state` is the post-block state.
fn build_block_template<S>(
    config: &BlockGenerationConfig,
    block_context: &BlockContext<'_>,
    parent_state: &Arc<S>,
    accumulated_batch: WriteBatch<S::AccountState>,
    output_buffer: ExecOutputBuffer,
    successful_txs: Vec<OLTransaction>,
    manifest_container: Option<OLL1ManifestContainer>,
) -> BlockAssemblyResult<(FullBlockTemplate, S)>
where
    S: BlockAssemblyStateAccess,
{
    // Clone parent state and apply accumulated batch to get state after transactions
    let mut final_state = parent_state.as_ref().clone();
    final_state.apply_write_batch(accumulated_batch)?;

    // Compute preseal state root (after transactions, before manifest processing)
    let preseal_state_root = final_state.compute_state_root()?;

    // For terminal blocks, process manifests to get final state root
    // For non-terminal blocks, preseal root IS the final root
    let (post_state_roots, l1_update) = if let Some(mc) = manifest_container {
        // Terminal block: process manifests to advance epoch and update state
        // Use the same output_buffer to accumulate logs from manifest processing
        let basic_ctx = BasicExecContext::new(*block_context.block_info(), &output_buffer);
        process_block_manifests(&mut final_state, &mc, &basic_ctx).map_err(|e| {
            error!(
                component = "ol_block_assembly",
                ?e,
                "manifest processing failed"
            );
            BlockAssemblyError::BlockConstruction(e)
        })?;

        let final_state_root = final_state.compute_state_root()?;
        let post_commitment =
            BlockPostStateCommitments::Terminal(preseal_state_root, final_state_root);
        let update = OLL1Update::new(preseal_state_root, mc);
        (post_commitment, Some(update))
    } else {
        // Non-terminal block: no manifest processing needed
        let post_commitment = BlockPostStateCommitments::Common(preseal_state_root);
        (post_commitment, None)
    };

    // Defense-in-depth: per-tx and manifest emission paths enforce the cap at
    // emit time, and this preserves an explicit terminal assembly invariant
    // check before finalizing the template.
    output_buffer
        .verify_logs_within_block_limit()
        .map_err(BlockAssemblyError::BlockConstruction)?;

    // Extract logs for computing logs root
    let logs = output_buffer.into_logs();

    // Build exec outputs to get header state root
    let exec_outputs = BlockExecOutputs::new(post_state_roots, logs);
    let logs_root = exec_outputs.compute_block_logs_root();
    let header_state_root = *exec_outputs.header_post_state_root();

    // Extract slot/epoch from block context
    let block_slot = block_context.slot();
    let block_epoch = block_context.epoch();
    let parent_blkid = block_context.compute_parent_blkid();

    // Build tx segment and body (terminal if l1_update is provided)
    let tx_segment = OLTxSegment::new(successful_txs)?;
    let body = OLBlockBody::new(tx_segment, l1_update);
    let body_root = body.compute_hash_commitment();

    // Set flags from body
    let mut flags = BlockFlags::zero();
    flags.set_is_terminal(body.is_body_terminal());

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
        .map_err(BlockAssemblyError::Acct)?
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
        .map_err(BlockAssemblyError::Acct)?
        .inbox_mmr()
        .num_entries();

    let mut all_acc_proofs = Vec::new();
    for check in proof_indexer.accumulator_checks() {
        match check {
            AccProofCheck::AsmHistory(claim) => {
                let proofs = proof_gen.generate_l1_header_proofs(slice::from_ref(claim), state)?;
                all_acc_proofs.extend(proofs.l1_headers_proofs().iter().cloned());
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
        manifests_mmr_entries = state.asm_manifests_mmr().num_entries(),
        "generated proofs for snark update via indexer"
    );

    let acc_proofs = RawMerkleProofList::from_vec_nonempty(all_acc_proofs);
    Ok(mempool_tx.with_accumulator_proofs(acc_proofs))
}

#[cfg(test)]
mod tests {
    use strata_acct_types::*;
    use strata_asm_manifest_types::AsmManifest;
    use strata_codec::encode_to_vec;
    use strata_identifiers::{Buf32, Buf64, L1BlockId, L1Height, OLBlockId, WtxidsRoot};
    use strata_msg_fmt::{Msg, OwnedMsg};
    use strata_ol_chain_types_new::{MAX_LOGS_PER_BLOCK, OLBlock, OLLog, SignedOLBlockHeader};
    use strata_ol_msg_types::{DEFAULT_OPERATOR_FEE, WITHDRAWAL_MSG_TYPE_ID, WithdrawalMsgData};
    use strata_ol_state_types::OLState;
    use strata_ol_stf::BRIDGE_GATEWAY_ACCT_ID;
    use strata_snark_acct_types::OutputMessage;
    use strata_storage::NodeStorage;

    use super::*;
    use crate::test_utils::*;

    #[test]
    fn test_l1_header_proof_gen_success() {
        let storage = create_test_storage();

        // Insert an ASM manifest hash into storage MMR and state MMR.
        let manifest = AsmManifest::new(
            1,
            L1BlockId::from(Buf32::from([1u8; 32])),
            WtxidsRoot::from(Buf32::zero()),
            vec![],
        )
        .expect("test manifest should be valid");
        let manifest_hash = manifest.compute_hash().into();
        let mut asm_mmr = StorageAsmMmr::new(storage.as_ref());
        asm_mmr.add_header(manifest_hash);

        // Create state with snark account
        let account_id = test_account_id(1);
        let mut state = create_test_genesis_state();
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);
        state.append_manifest(manifest.height(), manifest);

        // Create tx with claims from the tracker using builder
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_claims(asm_mmr.claims())
            .build();

        let ctx = create_test_context(storage.clone());

        // Convert transaction (generates accumulator proofs).
        let result = add_accumulator_proofs(&ctx, &state, mempool_tx);

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

    #[test]
    fn test_inbox_proof_gen_success() {
        let storage = create_test_storage();
        let mut state = create_test_genesis_state();

        // Create account
        let account_id = test_account_id(1);
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        // Use StorageInboxMmr to populate inbox messages
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(storage.as_ref(), account_id);
        inbox_mmr.add_messages(messages.clone());
        insert_inbox_messages_into_state(&mut state, account_id, &messages);

        // Create tx using builder
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_processed_messages(messages.clone())
            .build();

        let ctx = create_test_context(storage.clone());
        let result = add_accumulator_proofs(&ctx, &state, mempool_tx);

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

    #[test]
    fn test_l1_header_claim_hash_mismatch() {
        let storage = create_test_storage();

        // Insert a deterministic ASM manifest hash into storage MMR and state MMR.
        let manifest = AsmManifest::new(
            1,
            L1BlockId::from(Buf32::from([1u8; 32])),
            WtxidsRoot::from(Buf32::zero()),
            vec![],
        )
        .expect("test manifest should be valid");
        let manifest_hash = manifest.compute_hash().into();
        let mut asm_mmr = StorageAsmMmr::new(storage.as_ref());
        asm_mmr.add_header(manifest_hash);

        // Create claim with correct MMR index but WRONG hash (deterministic to guarantee mismatch)
        let wrong_hash = test_hash(99);
        assert_ne!(
            wrong_hash,
            asm_mmr.hashes()[0],
            "Test setup: wrong_hash should differ from actual hash"
        );

        let mmr_idx = asm_mmr.indices()[0];
        let invalid_claims = vec![AccumulatorClaim::new(mmr_idx, wrong_hash)];

        let account_id = test_account_id(1);
        let mut state = create_test_genesis_state();
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);
        state.append_manifest(manifest.height(), manifest);

        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_claims(invalid_claims)
            .build();
        let ctx = create_test_context(storage.clone());
        let result = add_accumulator_proofs(&ctx, &state, mempool_tx);
        assert!(result.is_err(), "Should fail with hash mismatch");
        let err = result.unwrap_err();
        assert!(
            matches!(err, BlockAssemblyError::L1HeaderHashMismatch { .. }),
            "Expected L1HeaderHashMismatch, got: {:?}",
            err
        );
    }

    #[test]
    fn test_l1_header_claim_missing_index() {
        let storage = create_test_storage();

        // Use StorageAsmMmr with random hashes
        let mut asm_mmr = StorageAsmMmr::new(storage.as_ref());
        asm_mmr.add_random_headers(1);

        // Create claim with non-existent index (index 999 doesn't exist)
        let nonexistent_index = 999u64;
        let invalid_claims = vec![AccumulatorClaim::new(
            nonexistent_index,
            asm_mmr.hashes()[0],
        )];

        let account_id = test_account_id(1);
        let mut state = create_test_genesis_state();
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_claims(invalid_claims)
            .build();
        let ctx = create_test_context(storage.clone());
        let result = add_accumulator_proofs(&ctx, &state, mempool_tx);

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

    #[test]
    fn test_l1_header_claim_empty_mmr() {
        // Setup storage WITHOUT any L1 headers in ASM MMR
        let storage = create_test_storage();

        // Create claim for MMR index 0 with arbitrary hash (MMR is empty)
        let arbitrary_hash = test_hash(42);
        let invalid_claims = vec![AccumulatorClaim::new(0, arbitrary_hash)];

        // Create state with snark account
        let account_id = test_account_id(1);
        let mut state = create_test_genesis_state();
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_claims(invalid_claims)
            .build();

        let ctx = create_test_context(storage.clone());
        // Conversion should fail with an index/range DB error.
        let result = add_accumulator_proofs(&ctx, &state, mempool_tx);

        assert!(result.is_err(), "Should fail when MMR is empty");
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                BlockAssemblyError::Db(DbError::MmrIndexOutOfRange { .. })
                    | BlockAssemblyError::Db(DbError::MmrLeafNotFound(_))
            ),
            "Expected Db(MmrIndexOutOfRange|MmrLeafNotFound), got: {:?}",
            err
        );
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

    #[test]
    fn test_inbox_claim_missing_index() {
        let storage = create_test_storage();
        let mut state = create_test_genesis_state();

        // Create account
        let account_id = test_account_id(1);
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        // Use StorageInboxMmr to add only 1 message
        let source_account = test_account_id(2);
        let messages = generate_message_entries(1, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(storage.as_ref(), account_id);
        inbox_mmr.add_messages(messages.clone());

        // Create transaction claiming to process messages at indices [5, 6]
        // which don't exist (only index 0 exists)
        let fake_messages = generate_message_entries(2, source_account);
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_processed_messages(fake_messages)
            .with_new_msg_idx(7) // Claims next_inbox_msg_idx = 7 after processing
            .build();

        insert_inbox_messages_into_state(&mut state, account_id, &messages);

        let ctx = create_test_context(storage.clone());
        let result = add_accumulator_proofs(&ctx, &state, mempool_tx);

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

    #[test]
    fn test_inbox_claim_invalid_msg_idx() {
        let storage = create_test_storage();
        let mut state = create_test_genesis_state();

        // Create account
        let account_id = test_account_id(1);
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        // Use StorageInboxMmr to add 2 messages
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(storage.as_ref(), account_id);
        inbox_mmr.add_messages(messages.clone());

        // Account has next_inbox_msg_idx = 0 on-chain
        // Create transaction claiming WRONG new next_inbox_msg_idx
        // Claims to process 2 messages but sets next_inbox_msg_idx = 10 (should be 2)
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_processed_messages(messages.clone())
            .with_new_msg_idx(10) // Wrong! Should be 2
            .build();

        insert_inbox_messages_into_state(&mut state, account_id, &messages);

        let ctx = create_test_context(storage.clone());
        let result = add_accumulator_proofs(&ctx, &state, mempool_tx);

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
    #[test]
    fn test_proof_ordering_l1_headers_before_inbox() {
        let storage = create_test_storage();
        let mut state = create_test_genesis_state();

        // Create account
        let account_id = test_account_id(1);
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        // Add L1 header to storage and state MMRs
        let manifest = AsmManifest::new(
            1,
            L1BlockId::from(Buf32::from([1u8; 32])),
            WtxidsRoot::from(Buf32::zero()),
            vec![],
        )
        .expect("test manifest should be valid");
        let manifest_hash = manifest.compute_hash().into();
        let mut asm_mmr = StorageAsmMmr::new(storage.as_ref());
        asm_mmr.add_header(manifest_hash);
        state.append_manifest(manifest.height(), manifest);

        // Add inbox messages to storage and state
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(storage.as_ref(), account_id);
        inbox_mmr.add_messages(messages.clone());
        insert_inbox_messages_into_state(&mut state, account_id, &messages);

        // Create tx with BOTH L1 claims and inbox messages
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_claims(asm_mmr.claims())
            .with_processed_messages(messages.clone())
            .build();

        let ctx = create_test_context(storage.clone());
        let tx = add_accumulator_proofs(&ctx, &state, mempool_tx)
            .expect("proof generation should succeed");
        let tx_proofs = tx.proofs();

        // Verify accumulator proofs exist and have correct count
        let acc_proofs = tx_proofs
            .accumulator_proofs()
            .expect("should have accumulator proofs");
        let n_l1 = asm_mmr.claims().len();
        let n_inbox = messages.len();
        assert_eq!(
            acc_proofs.proofs().len(),
            n_l1 + n_inbox,
            "Should have {n_l1} L1 header + {n_inbox} inbox = {} total accumulator proofs",
            n_l1 + n_inbox
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

    // Helper to validate terminal block with L1 updates
    fn check_terminal_block_with_manifests(
        block_template: &FullBlockTemplate,
        expected_heights: &[L1Height],
    ) {
        let body = block_template.body();
        let l1_update = body.l1_update();
        assert!(
            l1_update.is_some(),
            "Terminal block should contain L1 update"
        );

        let manifest_cont = l1_update.unwrap().manifest_cont();
        let manifests = manifest_cont.manifests();
        assert_eq!(
            manifests.len(),
            expected_heights.len(),
            "Should have {} L1 manifests",
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

    // Helper to validate non-terminal block without L1 updates
    fn check_non_terminal_block(block_template: &FullBlockTemplate) {
        let body = block_template.body();
        let l1_update = body.l1_update();
        assert!(
            l1_update.is_none(),
            "Non-terminal block should NOT contain L1 update"
        );
    }

    // Helper to build blocks from start_commitment up to (but not including) target_slot.
    // Stores blocks and states so subsequent blocks can find their parent.
    async fn build_blocks_to_slot<C, E>(
        start_commitment: OLBlockCommitment,
        target_slot: u64,
        ctx: &C,
        storage: &NodeStorage,
        epoch_sealing_policy: &E,
    ) -> OLBlockCommitment
    where
        C: BlockAssemblyAnchorContext<State = OLState> + AccumulatorProofGenerator,
        E: EpochSealingPolicy,
    {
        let mut current_commitment = start_commitment;

        let start_slot = if current_commitment.is_null() {
            0
        } else {
            start_commitment.slot() + 1
        };

        for slot in start_slot..target_slot {
            let config = BlockGenerationConfig::new(current_commitment);

            // Fetch parent state
            let parent_state = ctx
                .fetch_state_for_tip(config.parent_block_commitment())
                .await
                .unwrap_or_else(|e| panic!("Failed to fetch parent state at slot {slot}: {e:?}"))
                .unwrap_or_else(|| panic!("Missing parent state at slot {slot}"));

            // Calculate slot and epoch
            let (block_slot, block_epoch) = calculate_block_slot_and_epoch(
                &config.parent_block_commitment(),
                parent_state.as_ref(),
            );

            // Construct block (no mempool txs for helper)
            let output = construct_block(
                ctx,
                epoch_sealing_policy,
                &config,
                parent_state,
                block_slot,
                block_epoch,
                vec![],
                AccumulatedDaData::new_empty(),
            )
            .await
            .unwrap_or_else(|e| panic!("Block construction at slot {slot} failed: {e:?}"));

            // Create commitment from header
            let header = output.template.header();
            let new_commitment = OLBlockCommitment::new(header.slot(), header.compute_blkid());

            // Store block (with dummy signature)
            let signed_header = SignedOLBlockHeader::new(header.clone(), Buf64::zero());
            let block = OLBlock::new(signed_header, output.template.body().clone());
            storage
                .ol_block()
                .put_block_data_async(block)
                .await
                .unwrap_or_else(|e| panic!("Failed to store block at slot {slot}: {e:?}"));

            // Store post-state at new commitment
            storage
                .ol_state()
                .put_toplevel_ol_state_async(new_commitment, output.post_state)
                .await
                .unwrap_or_else(|e| panic!("Failed to store state at slot {slot}: {e:?}"));

            current_commitment = new_commitment;
        }

        current_commitment
    }

    #[tokio::test(flavor = "multi_thread")]
    #[should_panic(expected = "generate_block_template_inner called with null parent")]
    async fn test_block_assembly_panics_on_null_parent() {
        let env = TestEnvBuilder::new().build().await;

        let (ctx, _mempool) = create_test_block_assembly_context(env.storage.clone());

        let config = BlockGenerationConfig::new(env.parent_commitment);
        let _ = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_non_terminal_block_at_slot_1() {
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .build()
            .await;

        let (ctx, _mempool) = create_test_block_assembly_context(env.storage);

        let config = BlockGenerationConfig::new(env.parent_commitment);
        let result = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await;
        assert!(
            result.is_ok(),
            "Block generation should succeed: {:?}",
            result.err()
        );

        let block_template = result.unwrap().into_template();
        check_block_slot_epoch(&block_template, 1, 1);
        check_non_terminal_block(&block_template);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_block_template_fallback_timestamp_uses_milliseconds() {
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .build()
            .await;

        let (ctx, _mempool) = create_test_block_assembly_context(env.storage);
        let config = BlockGenerationConfig::new(env.parent_commitment);
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let result = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
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
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .build()
            .await;

        let (ctx, _mempool) = create_test_block_assembly_context(env.storage.clone());

        let current_commitment = build_blocks_to_slot(
            env.parent_commitment,
            10,
            &ctx,
            env.storage.as_ref(),
            &env.epoch_sealing_policy,
        )
        .await;

        let config = BlockGenerationConfig::new(current_commitment);
        let result = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await;
        assert!(
            result.is_ok(),
            "Block generation should succeed: {:?}",
            result.err()
        );

        let block_template = result.unwrap().into_template();
        check_block_slot_epoch(&block_template, 10, 1);
        // After genesis processes manifest 1, only manifests 2 and 3 remain
        check_terminal_block_with_manifests(&block_template, &[2, 3]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_terminal_block_manifest_boundary_from_last_l1_height() {
        // Set last_l1_height to 2, but only provide manifests starting at 3.
        let env = TestEnvBuilder::new()
            .with_parent_slot(1)
            .with_claim_manifests(2)
            .with_asm_manifests(&[3, 4])
            .build()
            .await;

        let (ctx, _mempool) = create_test_block_assembly_context(env.storage.clone());

        let current_commitment = build_blocks_to_slot(
            env.parent_commitment,
            10,
            &ctx,
            env.storage.as_ref(),
            &env.epoch_sealing_policy,
        )
        .await;

        let config = BlockGenerationConfig::new(current_commitment);
        let result = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("Block generation should succeed");

        let block_template = result.into_template();
        check_terminal_block_with_manifests(&block_template, &[3, 4]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_non_terminal_block_at_slot_11() {
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .build()
            .await;

        let (ctx, _mempool) = create_test_block_assembly_context(env.storage.clone());

        let current_commitment = build_blocks_to_slot(
            env.parent_commitment,
            11,
            &ctx,
            env.storage.as_ref(),
            &env.epoch_sealing_policy,
        )
        .await;

        let config = BlockGenerationConfig::new(current_commitment);
        let result = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await;
        assert!(
            result.is_ok(),
            "Block generation should succeed: {:?}",
            result.err()
        );

        let block_template = result.unwrap().into_template();
        check_block_slot_epoch(&block_template, 11, 2);
        check_non_terminal_block(&block_template);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_valid_tx_included_in_block() {
        // Setup env with snark account (seq_no=0 initially)
        let account_id = test_account_id(1);
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .with_account(account_id, DEFAULT_ACCOUNT_BALANCE)
            .build()
            .await;

        // Setup inbox MMR with real messages using StorageInboxMmr
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(&env.storage, account_id);
        inbox_mmr.add_messages(messages.clone());

        // Create tx and add to mock provider
        let valid_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_processed_messages(messages.clone())
            .build();
        let txid = valid_tx.compute_txid();

        let (ctx, mempool) = create_test_block_assembly_context(env.storage.clone());

        insert_inbox_messages_into_storage_state(
            env.storage.as_ref(),
            env.parent_commitment,
            account_id,
            &messages,
        )
        .await;
        mempool.add_transaction(txid, valid_tx);

        let config = BlockGenerationConfig::new(env.parent_commitment);
        let output = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("Block generation should succeed");

        // Assert: tx included in block
        let txs = output
            .template()
            .body()
            .tx_segment()
            .expect("Should have tx segment")
            .txs();
        assert_eq!(txs.len(), 1, "Block should contain 1 transaction");
        assert_eq!(
            txs[0].target(),
            Some(account_id),
            "Included tx should target the expected account"
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

        let result =
            BlockTemplateResult::new(template, failed_txs.clone(), AccumulatedDaData::new_empty());
        let (out_template, out_failed, _da) = result.into_parts();

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
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .with_account(account1, DEFAULT_ACCOUNT_BALANCE)
            .with_account(account2, DEFAULT_ACCOUNT_BALANCE)
            .build()
            .await;

        // Generate messages and insert into account1's inbox MMR using StorageInboxMmr
        let source_account = test_account_id(3);
        let real_messages = generate_message_entries(2, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(&env.storage, account1);
        inbox_mmr.add_messages(real_messages.clone());

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
        let (ctx, mempool) = create_test_block_assembly_context(env.storage.clone());

        insert_inbox_messages_into_storage_state(
            env.storage.as_ref(),
            env.parent_commitment,
            account1,
            &real_messages,
        )
        .await;
        mempool.add_transaction(valid_txid, valid_tx);
        mempool.add_transaction(invalid_txid, invalid_tx);

        let config = BlockGenerationConfig::new(env.parent_commitment);
        let output = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("Block generation should succeed");

        // Assert: block has 1 transaction (valid included, invalid rejected)
        let txs = output
            .template()
            .body()
            .tx_segment()
            .expect("Should have tx segment")
            .txs();
        assert_eq!(
            txs.len(),
            1,
            "Block should contain 1 tx (valid included, invalid rejected)"
        );
        // Verify the included tx is for account1 (the valid one)
        assert_eq!(
            txs[0].target(),
            Some(account1),
            "Included tx should be from account1 (valid tx)"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_l1_header_mmr_claims() {
        // Setup env with two snark accounts and manifests in both state and storage MMRs
        let account1 = test_account_id(1);
        let account2 = test_account_id(2);
        let env = TestEnvBuilder::new()
            .with_parent_slot(1) // Start from slot 1 instead of genesis to avoid genesis manifest conflicts
            .with_account(account1, DEFAULT_ACCOUNT_BALANCE)
            .with_account(account2, DEFAULT_ACCOUNT_BALANCE)
            .with_claim_manifests(2)
            .build()
            .await;

        // Valid tx for account1: L1 header claims exist in both MMRs (using MMR leaf index)
        let valid_claims = vec![AccumulatorClaim::new(
            env.manifests[0].mmr_idx,
            env.manifests[0].hash,
        )];
        let valid_tx = MempoolSnarkTxBuilder::new(account1)
            .with_seq_no(0)
            .with_l1_claims(valid_claims)
            .build();
        let valid_txid = valid_tx.compute_txid();

        // Invalid tx for account2: non-existent MMR index (no corresponding MMR leaf)
        let fake_hash = test_hash(99);
        let missing_height = env.manifests.last().unwrap().mmr_idx + 100;
        let invalid_claims = vec![AccumulatorClaim::new(missing_height, fake_hash)];
        let invalid_tx = MempoolSnarkTxBuilder::new(account2)
            .with_seq_no(0)
            .with_l1_claims(invalid_claims)
            .build();
        let invalid_txid = invalid_tx.compute_txid();

        // Build block
        let (ctx, mempool) = create_test_block_assembly_context(env.storage.clone());
        mempool.add_transaction(valid_txid, valid_tx);
        mempool.add_transaction(invalid_txid, invalid_tx);

        let config = BlockGenerationConfig::new(env.parent_commitment);
        let output = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("Block generation should succeed");

        // Assert: block has 1 transaction (valid included, invalid rejected)
        let txs = output
            .template()
            .body()
            .tx_segment()
            .expect("Should have tx segment")
            .txs();
        assert_eq!(
            txs.len(),
            1,
            "Block should contain 1 tx (valid included, invalid rejected)"
        );
        // Verify the included tx is for account1 (the valid one)
        assert_eq!(
            txs[0].target(),
            Some(account1),
            "Included tx should be from account1 (valid tx)"
        );
    }

    /// Tests that dependent transactions with sequential seq_no are both included.
    /// tx1: seq_no=0, tx2: seq_no=1
    /// Both should succeed because tx2 sees tx1's state changes (seq_no incremented to 1).
    #[tokio::test(flavor = "multi_thread")]
    async fn test_sequential_seq_no_both_succeed() {
        let account_id = test_account_id(1);
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .with_account(account_id, DEFAULT_ACCOUNT_BALANCE)
            .build()
            .await;

        // Setup inbox MMR with messages for both txs using StorageInboxMmr
        let source_account = test_account_id(2);
        let messages = generate_message_entries(4, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(&env.storage, account_id);
        inbox_mmr.add_messages(messages.clone());

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
        let (ctx, mempool) = create_test_block_assembly_context(env.storage.clone());

        insert_inbox_messages_into_storage_state(
            env.storage.as_ref(),
            env.parent_commitment,
            account_id,
            &messages,
        )
        .await;
        mempool.add_transaction(tx1_id, tx1);
        mempool.add_transaction(tx2_id, tx2);

        let config = BlockGenerationConfig::new(env.parent_commitment);
        let output = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("Block generation should succeed");

        // Assert: both txs included (tx2 succeeds because it sees tx1's seq_no increment)
        let txs = output
            .template()
            .body()
            .tx_segment()
            .expect("Should have tx segment")
            .txs();
        assert_eq!(
            txs.len(),
            2,
            "Block should contain both txs (tx2 sees tx1's state changes)"
        );
    }

    /// Tests that tx with seq_no=1 fails if tx with seq_no=0 is not present.
    /// Only tx2 (seq_no=1) submitted - should fail during execution because account has seq_no=0.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_dependent_tx_fails_without_predecessor() {
        let account_id = test_account_id(1);
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .with_account(account_id, DEFAULT_ACCOUNT_BALANCE)
            .build()
            .await;

        // Setup inbox MMR with messages using StorageInboxMmr
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(&env.storage, account_id);
        inbox_mmr.add_messages(messages.clone());

        // Only submit tx with seq_no=1 (no seq_no=0 predecessor)
        // Block assembly will reject during execution because account has seq_no=0
        let tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(1)
            .with_processed_messages(messages.clone())
            .build();
        let txid = tx.compute_txid();

        let (ctx, mempool) = create_test_block_assembly_context(env.storage.clone());

        insert_inbox_messages_into_storage_state(
            env.storage.as_ref(),
            env.parent_commitment,
            account_id,
            &messages,
        )
        .await;
        mempool.add_transaction(txid, tx);

        let config = BlockGenerationConfig::new(env.parent_commitment);
        let output = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("Block generation should succeed");

        // Block should have no txs - the seq_no=1 tx is rejected during execution
        let tx_segment = output.template().body().tx_segment();
        let tx_count = tx_segment.map(|seg| seg.txs().len()).unwrap_or(0);
        assert_eq!(
            tx_count, 0,
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
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .with_account(account1, DEFAULT_ACCOUNT_BALANCE)
            .with_account(account2, DEFAULT_ACCOUNT_BALANCE)
            .build()
            .await;

        // Setup inbox MMRs for both accounts using StorageInboxMmr
        let source_account = test_account_id(3);
        let account1_messages = generate_message_entries(2, source_account);
        let mut inbox_mmr1 = StorageInboxMmr::new(&env.storage, account1);
        inbox_mmr1.add_messages(account1_messages.clone());

        let account2_messages = generate_message_entries(2, source_account);
        let mut inbox_mmr2 = StorageInboxMmr::new(&env.storage, account2);
        inbox_mmr2.add_messages(account2_messages.clone());

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
        let (ctx, mempool) = create_test_block_assembly_context(env.storage.clone());

        insert_inbox_messages_into_storage_state(
            env.storage.as_ref(),
            env.parent_commitment,
            account1,
            &account1_messages,
        )
        .await;
        insert_inbox_messages_into_storage_state(
            env.storage.as_ref(),
            env.parent_commitment,
            account2,
            &account2_messages,
        )
        .await;
        mempool.add_transaction(tx1_id, tx1);
        mempool.add_transaction(tx2_id, tx2);

        let config = BlockGenerationConfig::new(env.parent_commitment);
        let output = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("Block generation should succeed");

        // Assert: tx2 included, tx1 rejected
        let tx_segment = output.template().body().tx_segment();
        let tx_count = tx_segment.map(|seg| seg.txs().len()).unwrap_or(0);
        assert_eq!(tx_count, 1, "Block should contain tx2 only");

        // Assert: tx1 removed (ConsensusInvalid), tx2 still in mempool (included txs not
        // auto-removed)
        let remaining = mempool.get_transactions(10).await.unwrap();
        assert_eq!(
            remaining.len(),
            1,
            "tx1 removed as invalid, tx2 still in mempool until block applied"
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
        let env = TestEnvBuilder::new()
            .with_parent_slot(0)
            .with_asm_manifests(&[1, 2, 3])
            .with_account(account1, 10000)
            .with_account(account2, 0)
            .with_account(account3, 0)
            .build()
            .await;

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
        let (ctx, mempool) = create_test_block_assembly_context(env.storage);
        mempool.add_transaction(tx1_id, tx1);
        mempool.add_transaction(tx2_id, tx2);

        let config = BlockGenerationConfig::new(env.parent_commitment);
        let output = generate_block_template_inner(
            &ctx,
            &env.epoch_sealing_policy,
            &env.sequencer_config,
            config,
            AccumulatedDaData::new_empty(),
        )
        .await
        .expect("Block generation should succeed");

        // Assert: both txs included
        // tx1 executes first, transferring 1000 to account2
        // tx2 executes second, account2 now has 1000 and can send 500
        let txs = output
            .template()
            .body()
            .tx_segment()
            .expect("Should have tx segment")
            .txs();
        assert_eq!(
            txs.len(),
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
    #[tokio::test(flavor = "multi_thread")]
    async fn test_tx_deferred_when_logs_would_overflow_remaining_budget() {
        let account_id = test_account_id(1);
        let storage = create_test_storage();
        let mut parent_state = create_test_genesis_state();
        add_snark_account_to_state(&mut parent_state, account_id, 1, DEFAULT_ACCOUNT_BALANCE);

        let (ctx, _mempool) = create_test_block_assembly_context(storage);
        let parent_header = create_test_parent_header();
        let block_info = BlockInfo::new(1_000_001, parent_header.slot() + 1, parent_header.epoch());
        let block_context = BlockContext::new(&block_info, Some(&parent_header));
        let accumulated_batch = WriteBatch::new_from_state(&parent_state);

        let withdrawal_msg_data = WithdrawalMsgData::new(
            DEFAULT_OPERATOR_FEE,
            b"bc1qlogcapoverflow".to_vec(),
            u32::MAX,
        )
        .expect("valid withdrawal data");
        let encoded_withdrawal_body =
            encode_to_vec(&withdrawal_msg_data).expect("encode withdrawal body");
        let withdrawal_msg =
            OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_withdrawal_body).expect("msg format");
        let withdrawal_payload_data = withdrawal_msg.to_vec();

        // Create a tx with 5 withdrawal messages (= 5 logs).
        let output_messages: Vec<_> = (0..5)
            .map(|_| {
                let payload = MsgPayload::new(
                    BitcoinAmount::from_sat(100_000_000),
                    withdrawal_payload_data.clone(),
                );
                OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, payload)
            })
            .collect();

        let tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_output_messages(output_messages)
            .build();
        let txid = tx.compute_txid();

        // Pre-fill output buffer so only 3 logs remain before the cap.
        // The tx produces 5 logs, so 5 > 3 remaining → soft-break.
        let output_buffer = ExecOutputBuffer::new_empty();
        let prefill = MAX_LOGS_PER_BLOCK as usize - 3;
        output_buffer
            .emit_logs((0..prefill).map(|i| OLLog::new(AccountSerial::from(i as u32), vec![])))
            .expect("pre-fill should succeed");

        let out = process_transactions(
            &ctx,
            &block_context,
            &output_buffer,
            &parent_state,
            accumulated_batch,
            vec![(txid, tx)],
            AccumulatedDaData::new_empty(),
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
    async fn test_block_full_log_overflow_tx_not_reported_invalid_and_kept_in_mempool() {
        let account_id = test_account_id(8);
        let storage = create_test_storage();
        let mut parent_state = create_test_genesis_state();
        add_snark_account_to_state(&mut parent_state, account_id, 1, DEFAULT_ACCOUNT_BALANCE);

        let (ctx, _mempool) = create_test_block_assembly_context(storage);
        let parent_header = create_test_parent_header();
        let block_info = BlockInfo::new(1_000_001, parent_header.slot() + 1, parent_header.epoch());
        let block_context = BlockContext::new(&block_info, Some(&parent_header));
        let accumulated_batch = WriteBatch::new_from_state(&parent_state);

        let withdrawal_msg_data =
            WithdrawalMsgData::new(DEFAULT_OPERATOR_FEE, b"bc1qlogcapfull".to_vec(), u32::MAX)
                .expect("valid withdrawal data");
        let encoded_withdrawal_body =
            encode_to_vec(&withdrawal_msg_data).expect("encode withdrawal body");
        let withdrawal_msg =
            OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_withdrawal_body).expect("msg format");
        let withdrawal_payload_data = withdrawal_msg.to_vec();

        // First tx: 10 withdrawal messages = 10 logs.
        let fill_messages: Vec<_> = (0..10)
            .map(|_| {
                let payload = MsgPayload::new(
                    BitcoinAmount::from_sat(100_000_000),
                    withdrawal_payload_data.clone(),
                );
                OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, payload)
            })
            .collect();
        let tx_fill = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_output_messages(fill_messages)
            .build();
        let tx_fill_id = tx_fill.compute_txid();

        // Second tx: 1 withdrawal message = 1 log.
        let tx_overflow = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(1)
            .with_output_messages(vec![{
                let payload = MsgPayload::new(
                    BitcoinAmount::from_sat(100_000_000),
                    withdrawal_payload_data.clone(),
                );
                OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, payload)
            }])
            .build();
        let tx_overflow_id = tx_overflow.compute_txid();

        // Pre-fill the buffer so that tx_fill's 10 logs exactly reach the cap,
        // leaving no room for tx_overflow's 1 log.
        let output_buffer = ExecOutputBuffer::new_empty();
        let prefill = MAX_LOGS_PER_BLOCK as usize - 10;
        output_buffer
            .emit_logs((0..prefill).map(|i| OLLog::new(AccountSerial::from(i as u32), vec![])))
            .expect("pre-fill should succeed");

        let out = process_transactions(
            &ctx,
            &block_context,
            &output_buffer,
            &parent_state,
            accumulated_batch,
            vec![(tx_fill_id, tx_fill), (tx_overflow_id, tx_overflow)],
            AccumulatedDaData::new_empty(),
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
    async fn test_process_transactions_soft_breaks_when_log_cap_already_reached() {
        let account_id = test_account_id(7);
        let storage = create_test_storage();
        let mut parent_state = create_test_genesis_state();
        add_snark_account_to_state(&mut parent_state, account_id, 1, DEFAULT_ACCOUNT_BALANCE);

        let (ctx, _mempool) = create_test_block_assembly_context(storage);
        let parent_header = create_test_parent_header();
        let block_info = BlockInfo::new(1_000_001, parent_header.slot() + 1, parent_header.epoch());
        let block_context = BlockContext::new(&block_info, Some(&parent_header));
        let accumulated_batch = WriteBatch::new_from_state(&parent_state);

        let withdrawal_msg_data = WithdrawalMsgData::new(
            DEFAULT_OPERATOR_FEE,
            b"bc1qlogcapreached".to_vec(),
            u32::MAX,
        )
        .expect("valid withdrawal data");
        let encoded_withdrawal_body =
            encode_to_vec(&withdrawal_msg_data).expect("encode withdrawal body");
        let withdrawal_msg =
            OwnedMsg::new(WITHDRAWAL_MSG_TYPE_ID, encoded_withdrawal_body).expect("msg format");
        let withdrawal_payload_data = withdrawal_msg.to_vec();

        let tx = MempoolSnarkTxBuilder::new(account_id)
            .with_seq_no(0)
            .with_output_messages(vec![{
                let payload = MsgPayload::new(
                    BitcoinAmount::from_sat(100_000_000),
                    withdrawal_payload_data.clone(),
                );
                OutputMessage::new(BRIDGE_GATEWAY_ACCT_ID, payload)
            }])
            .build();
        let txid = tx.compute_txid();

        // Pre-fill the block output buffer to exactly the cap so the next tx that emits logs
        // hits the soft-break branch without being marked invalid.
        let output_buffer = ExecOutputBuffer::new_empty();
        output_buffer
            .emit_logs(
                (0..MAX_LOGS_PER_BLOCK).map(|i| OLLog::new(AccountSerial::from(i as u32), vec![])),
            )
            .expect("pre-filling up to the cap should succeed");

        let out = process_transactions(
            &ctx,
            &block_context,
            &output_buffer,
            &parent_state,
            accumulated_batch,
            vec![(txid, tx)],
            AccumulatedDaData::new_empty(),
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
}
