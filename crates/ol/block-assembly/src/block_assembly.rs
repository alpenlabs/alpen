//! Block assembly logic.

use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use strata_config::SequencerConfig;
use strata_db_types::errors::DbError;
use strata_identifiers::{Epoch, OLBlockCommitment, OLTxId, Slot};
use strata_ledger_types::{IAccountStateConstructible, IAccountStateMut, IStateAccessor};
use strata_ol_chain_types_new::{
    BlockFlags, OLBlockBody, OLBlockHeader, OLL1ManifestContainer, OLL1Update, OLTransaction,
    OLTxSegment, SnarkAccountUpdateTxPayload, TransactionPayload,
};
use strata_ol_mempool::{
    MempoolTxInvalidReason, OLMempoolSnarkAcctUpdateTxPayload, OLMempoolTransaction,
    OLMempoolTxPayload,
};
use strata_ol_state_support_types::WriteTrackingState;
use strata_ol_state_types::{IStateBatchApplicable, WriteBatch};
use strata_ol_stf::{
    BasicExecContext, BlockContext, BlockExecOutputs, BlockInfo, BlockPostStateCommitments,
    ExecError, ExecOutputBuffer, TxExecContext, process_block_manifests, process_block_start,
    process_epoch_initial, process_single_tx,
};
use strata_snark_acct_types::{SnarkAccountUpdateContainer, UpdateAccumulatorProofs};
use tracing::{debug, error};

use crate::{
    AccumulatorProofGenerator, BlockAssemblyResult, BlockAssemblyStateAccess, EpochSealingPolicy,
    MempoolProvider,
    context::BlockAssemblyAnchorContext,
    error::BlockAssemblyError,
    types::{BlockGenerationConfig, BlockTemplateResult, FailedMempoolTx, FullBlockTemplate},
};

/// Output from processing transactions during block assembly.
struct ProcessTransactionsOutput<S: IStateAccessor> {
    /// Transactions that passed validation and execution.
    successful_txs: Vec<OLTransaction>,
    /// Transactions that failed during block assembly.
    failed_txs: Vec<FailedMempoolTx>,
    /// Accumulated write batch after processing all transactions.
    accumulated_batch: WriteBatch<S::AccountState>,
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
        | ExecError::Acct(_) => MempoolTxInvalidReason::Invalid,

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
        | BlockAssemblyError::L1HeaderLeafNotFound(_)
        | BlockAssemblyError::InboxLeafNotFound { .. }
        | BlockAssemblyError::InboxEntryHashMismatch { .. }
        | BlockAssemblyError::InvalidMmrRange { .. }
        | BlockAssemblyError::AccountNotFound(_)
        | BlockAssemblyError::InboxProofCountMismatch { .. } => MempoolTxInvalidReason::Invalid,

        // Block assembly internal errors (not consensus-related)
        BlockAssemblyError::BlockConstruction(_)
        | BlockAssemblyError::Database(_)
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
        | BlockAssemblyError::CannotBuildGenesis => MempoolTxInvalidReason::Failed,
    }
}

/// Output from block construction containing the template, failed transactions, and final state.
#[expect(dead_code, reason = "Used in next commit")]
pub(crate) struct ConstructBlockOutput<S> {
    /// The constructed block template.
    pub(crate) template: FullBlockTemplate,
    /// Transactions that failed during block assembly.
    pub(crate) failed_txs: Vec<FailedMempoolTx>,
    /// The post state after applying all transactions.
    pub(crate) post_state: S,
}

/// Generate a block template (stub implementation).
///
/// This is a placeholder that will be fully implemented in the next commit.
/// For now, it returns an error to indicate that the implementation is pending.
///
/// Returns a [`BlockTemplateResult`] containing both the generated template and
/// any transactions that failed validation during assembly.
pub(crate) async fn generate_block_template_inner<C, E>(
    _ctx: &C,
    _epoch_sealing_policy: &E,
    _sequencer_config: &SequencerConfig,
    _block_generation_config: BlockGenerationConfig,
) -> BlockAssemblyResult<BlockTemplateResult>
where
    C: BlockAssemblyAnchorContext + AccumulatorProofGenerator + MempoolProvider,
    C::State: BlockAssemblyStateAccess,
    E: EpochSealingPolicy,
{
    Err(BlockAssemblyError::Database(DbError::Other(
        "Block assembly implementation pending".to_string(),
    )))
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
#[expect(dead_code, reason = "Used in next commit")]
fn calculate_block_slot_and_epoch<S: IStateAccessor>(
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
#[expect(dead_code, reason = "Used in next commit")]
async fn construct_block<C, E>(
    ctx: &C,
    epoch_sealing_policy: &E,
    config: &BlockGenerationConfig,
    parent_state: Arc<C::State>,
    block_slot: Slot,
    block_epoch: Epoch,
    mempool_txs: Vec<(OLTxId, OLMempoolTransaction)>,
) -> BlockAssemblyResult<ConstructBlockOutput<C::State>>
where
    C: BlockAssemblyAnchorContext + AccumulatorProofGenerator,
    E: EpochSealingPolicy,
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
        BlockAssemblyError::Database(DbError::Other(format!(
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
    let accumulated_batch = execute_block_initialization(parent_state.as_ref(), &block_context);

    // Phase 2: Process each transaction against accumulated state using AccumulatorProofGenerator.
    let ProcessTransactionsOutput {
        successful_txs,
        failed_txs,
        accumulated_batch,
    } = process_transactions(
        ctx,
        &block_context,
        &output_buffer,
        parent_state.as_ref(),
        accumulated_batch,
        mempool_txs,
    );

    // Phase 3: Detect terminal blocks and fetch L1 manifests if needed.
    let manifest_container = if epoch_sealing_policy.should_seal_epoch(block_slot) {
        fetch_asm_manifests_for_terminal_block(ctx, parent_state.as_ref()).await?
    } else {
        None
    };

    // Phase 4: Finalize block construction.
    let (template, post_state) = build_block_template(
        config,
        &block_context,
        &parent_state,
        accumulated_batch,
        output_buffer,
        successful_txs,
        manifest_container,
    )?;

    Ok(ConstructBlockOutput {
        template,
        failed_txs,
        post_state,
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
    let start_height = (last_l1_height + 1) as u64;

    // Fetch manifests using BlockAssemblyAnchorContext trait
    let manifests = ctx.fetch_asm_manifests_from(start_height).await?;

    if manifests.is_empty() {
        Ok(None)
    } else {
        let container = OLL1ManifestContainer::new(manifests)?;
        Ok(Some(container))
    }
}

/// Executes block initialization (epoch initial + block start) on a fresh write batch.
///
/// Returns the accumulated write batch containing initialization changes.
fn execute_block_initialization<S>(
    parent_state: &S,
    block_context: &BlockContext<'_>,
) -> WriteBatch<S::AccountState>
where
    S: IStateAccessor,
    S::AccountState: Clone + IAccountStateConstructible + IAccountStateMut,
{
    let mut accumulated_batch = WriteBatch::new_from_state(parent_state);

    let mut init_state = WriteTrackingState::new(parent_state, accumulated_batch.clone());

    // Process block start for every block (sets cur_slot, etc.)
    // Per spec: process_slot_start runs before process_epoch_initial.
    process_block_start(&mut init_state, block_context)
        .expect("block start processing should not fail");

    // Process epoch initial if this is the first block of the epoch.
    if block_context.is_epoch_initial() {
        let init_ctx = block_context.get_epoch_initial_context();
        process_epoch_initial(&mut init_state, &init_ctx)
            .expect("epoch initial processing should not fail");
    }

    accumulated_batch = init_state.into_batch();
    accumulated_batch
}

/// Processes transactions with per-tx staging, filtering out failed ones.
#[tracing::instrument(
    skip_all,
    fields(component = "ol_block_assembly", tx_count = mempool_txs.len())
)]
#[tracing::instrument(
    skip(proof_gen, output_buffer, parent_state, accumulated_batch, mempool_txs),
    fields(component = "ol_block_assembly")
)]
fn process_transactions<P, S>(
    proof_gen: &P,
    block_context: &BlockContext<'_>,
    output_buffer: &ExecOutputBuffer,
    parent_state: &S,
    accumulated_batch: WriteBatch<S::AccountState>,
    mempool_txs: Vec<(OLTxId, OLMempoolTransaction)>,
) -> ProcessTransactionsOutput<S>
where
    P: AccumulatorProofGenerator,
    S: IStateAccessor,
    S::AccountState: Clone + IAccountStateConstructible + IAccountStateMut,
{
    let mut successful_txs = Vec::new();
    let mut failed_txs = Vec::new();

    // Create staging state once, reuse across transactions.
    // We work directly on this state and only clone for backup before each tx.
    // On success: backup is discarded. On failure: restore from backup.
    let mut staging_state = WriteTrackingState::new(parent_state, accumulated_batch);

    for (txid, mempool_tx) in mempool_txs {
        // Step 1: Validate and generate accumulator proofs, convert to OL transaction.
        // This only reads from state, so no rollback needed on failure.
        let tx = match convert_mempool_tx_to_ol_tx(proof_gen, mempool_tx) {
            Ok(tx) => tx,
            Err(e) => {
                debug!(?txid, %e, "failed to validate/generate proofs for transaction");
                failed_txs.push((txid, block_assembly_error_to_mempool_reason(&e)));
                continue;
            }
        };

        // Step 2: Clone batch as backup before execution.
        let backup_batch = staging_state.batch().clone();

        // Step 3: Create per-tx output buffer and execute transaction.
        // Logs are only merged into main buffer on success; on failure they're discarded.
        let tx_buffer = ExecOutputBuffer::new_empty();
        let basic_ctx = BasicExecContext::new(*block_context.block_info(), &tx_buffer);
        let tx_ctx = TxExecContext::new(&basic_ctx, block_context.parent_header());

        match process_single_tx(&mut staging_state, &tx, &tx_ctx) {
            Ok(()) => {
                // Success: merge logs and keep state changes
                output_buffer.emit_logs(tx_buffer.into_logs());
                successful_txs.push(tx);
            }
            Err(e) => {
                // Failure: discard tx_buffer (logs) and restore state from backup
                debug!(?txid, %e, "transaction execution failed during staging");
                staging_state = WriteTrackingState::new(parent_state, backup_batch);
                failed_txs.push((txid, stf_exec_error_to_mempool_reason(&e)));
            }
        }
    }

    ProcessTransactionsOutput {
        successful_txs,
        failed_txs,
        accumulated_batch: staging_state.into_batch(),
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
    S: IStateBatchApplicable + IStateAccessor + Clone,
    S::AccountState: Clone + IAccountStateConstructible + IAccountStateMut,
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
        let post_roots = BlockPostStateCommitments::Terminal(preseal_state_root, final_state_root);
        let update = OLL1Update::new(preseal_state_root, mc);
        (post_roots, Some(update))
    } else {
        // Non-terminal block: no manifest processing needed
        let post_roots = BlockPostStateCommitments::Common(preseal_state_root);
        (post_roots, None)
    };

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

    // Use timestamp from config if provided, otherwise compute from system time
    let timestamp = config.ts().unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_secs()
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

/// Convert a mempool transaction to a full OL transaction with accumulator proofs.
///
/// For SnarkAccountUpdate transactions, this:
/// 1. Validates message index against account state
/// 2. Generates MessageEntryProof for each message using `AccumulatorProofGenerator`
/// 3. Generates MmrEntryProof for each L1 header reference using `AccumulatorProofGenerator`
fn convert_mempool_tx_to_ol_tx<P: AccumulatorProofGenerator>(
    proof_gen: &P,
    mempool_tx: OLMempoolTransaction,
) -> BlockAssemblyResult<OLTransaction> {
    let attachment = mempool_tx.attachment().clone();

    let payload = match mempool_tx.payload() {
        OLMempoolTxPayload::GenericAccountMessage(gam) => {
            // Generic account messages don't need proofs
            TransactionPayload::GenericAccountMessage(gam.clone())
        }

        OLMempoolTxPayload::SnarkAccountUpdate(mempool_payload) => {
            convert_snark_account_update(proof_gen, mempool_payload)?
        }
    };

    Ok(OLTransaction::new(payload, attachment))
}

/// Converts a snark account update mempool payload to a full transaction payload.
///
/// Validates message index against account state and generates accumulator proofs.
fn convert_snark_account_update<P: AccumulatorProofGenerator>(
    proof_gen: &P,
    mempool_payload: &OLMempoolSnarkAcctUpdateTxPayload,
) -> BlockAssemblyResult<TransactionPayload> {
    let target = *mempool_payload.target();
    let base_update = mempool_payload.base_update().clone();
    let operation = base_update.operation();

    // Generate inbox message proofs using AccumulatorProofGenerator
    // Calculate where messages start: new_state points to NEXT unprocessed message,
    // so subtract the number of messages being processed in this transaction
    let messages = operation.processed_messages();
    let start_idx = operation
        .new_proof_state()
        .next_inbox_msg_idx()
        .saturating_sub(messages.len() as u64);
    let inbox_proofs = proof_gen.generate_inbox_proofs(target, messages, start_idx)?;

    // Generate L1 header proofs using AccumulatorProofGenerator
    let l1_header_refs = operation.ledger_refs().l1_header_refs();
    let l1_header_proofs = proof_gen.generate_l1_header_proofs(l1_header_refs)?;

    // Create accumulator proofs
    let accumulator_proofs = UpdateAccumulatorProofs::new(inbox_proofs, l1_header_proofs);

    // Convert to full container
    let update_container = SnarkAccountUpdateContainer::new(base_update, accumulator_proofs);

    // Create transaction payload
    let tx_payload = SnarkAccountUpdateTxPayload::new(target, update_container);
    Ok(TransactionPayload::SnarkAccountUpdate(tx_payload))
}

#[cfg(test)]
mod tests {
    use strata_acct_types::AcctError;
    use strata_identifiers::OLBlockId;
    use strata_ol_state_types::OLState;
    use strata_snark_acct_types::AccumulatorClaim;

    use super::*;
    use crate::test_utils::{
        MempoolSnarkTxBuilder, StorageAsmMmr, StorageInboxMmr, add_snark_account_to_state,
        create_test_context, create_test_storage, generate_message_entries, test_account_id,
        test_hash,
    };

    #[test]
    fn test_l1_header_proof_gen_success() {
        let storage = create_test_storage();

        // Use StorageAsmMmr to populate L1 headers with random hashes
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_random_headers(1);

        // Create state with snark account
        let account_id = test_account_id(1);
        let mut state = OLState::new_genesis();
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        // Create tx with claims from the tracker using builder
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_claims(asm_mmr.claims())
            .build();

        let ctx = create_test_context(storage.clone());

        // Convert mempool transaction to payload (generates proofs)
        let mempool_payload = match mempool_tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => payload,
            _ => panic!("Expected snark account update payload"),
        };
        let result = convert_snark_account_update(&ctx, mempool_payload);

        assert!(
            result.is_ok(),
            "Proof generation should succeed, got error: {:?}",
            result.as_ref().err()
        );

        let payload = result.unwrap();
        match payload {
            TransactionPayload::SnarkAccountUpdate(sau) => {
                let proofs = sau.update_container().accumulator_proofs();
                let l1_proofs = proofs.ledger_ref_proofs().l1_headers_proofs();

                assert_eq!(l1_proofs.len(), 1, "Should have 1 L1 header proof");
                assert_eq!(
                    l1_proofs[0].entry_hash(),
                    asm_mmr.hashes()[0],
                    "Proof should have correct entry hash"
                );
            }
            _ => panic!("Expected SnarkAccountUpdate transaction"),
        }
    }

    #[test]
    fn test_inbox_proof_gen_success() {
        let storage = create_test_storage();
        let mut state = OLState::new_genesis();

        // Create account
        let account_id = test_account_id(1);
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        // Use StorageInboxMmr to populate inbox messages
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(&storage, account_id);
        inbox_mmr.add_messages(messages.clone());

        // Create tx using builder
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_processed_messages(messages.clone())
            .build();

        let ctx = create_test_context(storage.clone());
        let mempool_payload = match mempool_tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => payload,
            _ => panic!("Expected snark account update payload"),
        };
        let result = convert_snark_account_update(&ctx, mempool_payload);

        assert!(
            result.is_ok(),
            "Proof generation should succeed, got error: {:?}",
            result.as_ref().err()
        );

        let payload = result.unwrap();
        match payload {
            TransactionPayload::SnarkAccountUpdate(payload) => {
                let proofs = payload.update_container().accumulator_proofs();
                let inbox_proofs = proofs.inbox_proofs();

                assert_eq!(inbox_proofs.len(), 2, "Should have 2 inbox message proofs");
                assert_eq!(
                    inbox_proofs[0].entry(),
                    &messages[0],
                    "First proof should have correct message entry"
                );
                assert_eq!(
                    inbox_proofs[1].entry(),
                    &messages[1],
                    "Second proof should have correct message entry"
                );
            }
            _ => panic!("Expected SnarkAccountUpdate transaction"),
        }
    }

    #[test]
    fn test_l1_header_claim_hash_mismatch() {
        let storage = create_test_storage();

        // Use StorageAsmMmr with random hashes
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_random_headers(1);

        // Create claim with correct index but WRONG hash (deterministic to guarantee mismatch)
        let wrong_hash = test_hash(99);
        assert_ne!(
            wrong_hash,
            asm_mmr.hashes()[0],
            "Test setup: wrong_hash should differ from actual hash"
        );

        let invalid_claims = vec![AccumulatorClaim::new(asm_mmr.indices()[0], wrong_hash)];

        let account_id = test_account_id(1);
        let mut state = OLState::new_genesis();
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_claims(invalid_claims)
            .build();
        let ctx = create_test_context(storage.clone());
        let mempool_payload = match mempool_tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => payload,
            _ => panic!("Expected snark account update payload"),
        };
        let result = convert_snark_account_update(&ctx, mempool_payload);

        assert!(result.is_err(), "Should fail with hash mismatch");
        let err = result.unwrap_err();
        assert!(
            matches!(&err, BlockAssemblyError::L1HeaderHashMismatch { .. }),
            "Expected L1HeaderHashMismatch, got: {:?}",
            err
        );
    }

    #[test]
    fn test_l1_header_claim_missing_index() {
        let storage = create_test_storage();

        // Use StorageAsmMmr with random hashes
        let mut asm_mmr = StorageAsmMmr::new(&storage);
        asm_mmr.add_random_headers(1);

        // Create claim with non-existent index (index 999 doesn't exist)
        let nonexistent_index = 999u64;
        let invalid_claims = vec![AccumulatorClaim::new(
            nonexistent_index,
            asm_mmr.hashes()[0],
        )];

        let account_id = test_account_id(1);
        let mut state = OLState::new_genesis();
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_claims(invalid_claims)
            .build();
        let ctx = create_test_context(storage.clone());
        let mempool_payload = match mempool_tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => payload,
            _ => panic!("Expected snark account update payload"),
        };
        let result = convert_snark_account_update(&ctx, mempool_payload);

        assert!(result.is_err(), "Should fail with nonexistent index");
        let err = result.unwrap_err();
        assert!(
            matches!(&err, BlockAssemblyError::L1HeaderLeafNotFound(_)),
            "Expected L1HeaderLeafNotFound, got: {:?}",
            err
        );
    }

    #[test]
    fn test_l1_header_claim_empty_mmr() {
        // Setup storage WITHOUT any L1 headers in ASM MMR
        let storage = create_test_storage();

        // Create claim for index 0 with arbitrary hash (MMR is empty)
        let arbitrary_hash = test_hash(42);
        let invalid_claims = vec![AccumulatorClaim::new(0, arbitrary_hash)];

        // Create state with snark account
        let account_id = test_account_id(1);
        let mut state = OLState::new_genesis();
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_l1_claims(invalid_claims)
            .build();

        let ctx = create_test_context(storage.clone());
        // Conversion should fail with L1HeaderLeafNotFound
        let mempool_payload = match mempool_tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => payload,
            _ => panic!("Expected snark account update payload"),
        };
        let result = convert_snark_account_update(&ctx, mempool_payload);

        assert!(result.is_err(), "Should fail when MMR is empty");
        let err = result.unwrap_err();
        assert!(
            matches!(err, BlockAssemblyError::L1HeaderLeafNotFound(_)),
            "Expected L1HeaderLeafNotFound, got: {:?}",
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

        // Verify Database errors map to Failed (infrastructure error)
        let db_err = BlockAssemblyError::Database(DbError::Other("test error".to_string()));
        let reason = block_assembly_error_to_mempool_reason(&db_err);
        assert!(
            matches!(reason, MempoolTxInvalidReason::Failed),
            "Database errors should map to Failed, got: {:?}",
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
        let mut state = OLState::new_genesis();

        // Create account
        let account_id = test_account_id(1);
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        // Use StorageInboxMmr to add only 1 message
        let source_account = test_account_id(2);
        let messages = generate_message_entries(1, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(&storage, account_id);
        inbox_mmr.add_messages(messages);

        // Create transaction claiming to process messages at indices [5, 6]
        // which don't exist (only index 0 exists)
        let fake_messages = generate_message_entries(2, source_account);
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_processed_messages(fake_messages)
            .with_new_msg_idx(7) // Claims next_inbox_msg_idx = 7 after processing
            .build();

        let ctx = create_test_context(storage.clone());
        let mempool_payload = match mempool_tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => payload,
            _ => panic!("Expected snark account update payload"),
        };
        let result = convert_snark_account_update(&ctx, mempool_payload);

        assert!(
            result.is_err(),
            "Should fail when claiming inbox messages that don't exist"
        );
        let err = result.unwrap_err();
        // Could be InboxLeafNotFound/InboxEntryHashMismatch from MMR or Acct error
        let reason = block_assembly_error_to_mempool_reason(&err);
        assert!(
            matches!(reason, MempoolTxInvalidReason::Invalid),
            "Expected Invalid mempool reason for missing inbox claims, got: {:?}",
            reason
        );
    }

    #[test]
    fn test_inbox_claim_invalid_msg_idx() {
        let storage = create_test_storage();
        let mut state = OLState::new_genesis();

        // Create account
        let account_id = test_account_id(1);
        add_snark_account_to_state(&mut state, account_id, 1, 100_000);

        // Use StorageInboxMmr to add 2 messages
        let source_account = test_account_id(2);
        let messages = generate_message_entries(2, source_account);
        let mut inbox_mmr = StorageInboxMmr::new(&storage, account_id);
        inbox_mmr.add_messages(messages.clone());

        // Account has next_inbox_msg_idx = 0 on-chain
        // Create transaction claiming WRONG new next_inbox_msg_idx
        // Claims to process 2 messages but sets next_inbox_msg_idx = 10 (should be 2)
        let mempool_tx = MempoolSnarkTxBuilder::new(account_id)
            .with_processed_messages(messages)
            .with_new_msg_idx(10) // Wrong! Should be 2
            .build();

        let ctx = create_test_context(storage.clone());
        let mempool_payload = match mempool_tx.payload() {
            OLMempoolTxPayload::SnarkAccountUpdate(payload) => payload,
            _ => panic!("Expected snark account update payload"),
        };
        let result = convert_snark_account_update(&ctx, mempool_payload);

        assert!(
            result.is_err(),
            "Should fail with invalid message index claim"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(
                err,
                BlockAssemblyError::Acct(AcctError::InvalidMsgIndex { .. })
                    | BlockAssemblyError::InboxEntryHashMismatch { .. }
                    | BlockAssemblyError::InboxLeafNotFound { .. }
            ),
            "Expected Acct(InvalidMsgIndex) or inbox MMR error, got: {:?}",
            err
        );
    }
}
