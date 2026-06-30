//! EE rollback maintenance commands.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
};

use alloy_eips::eip2718::Encodable2718 as _;
use alpen_ee_common::{
    encode_batch_task_key, encode_chunk_task_key, Batch, BatchId, BatchStatus, BatchStorage, Chunk,
    ChunkId, ChunkStatus, ChunkStorage, EnginePayload, ExecBlockPayload, ExecBlockStorage,
    OLBlockOrEpoch, Storage, StorageError,
};
use alpen_ee_database::{EeNodeStorage, EeProverDbSled};
use alpen_reth_node::AlpenBuiltPayload;
use argh::FromArgs;
use reth_node_api::BuiltPayload as _;
use reth_primitives_traits::SignedTransaction as _;
use strata_acct_types::Hash;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::prover_task::ProverTaskDatabase;

use crate::{
    cli::OutputFormat,
    cmd::prover_task_common::print_force_hint,
    output::{
        ee_revert::{
            AcceptedFrontierInfo, AccountStateRollbackInfo, AffectedBatchInfo, AffectedBlockInfo,
            AffectedBlockSummary, AffectedChunkInfo, BlockRangeSummary, BlockTransactionInfo,
            EeRevertBatchesReport, MutationInfo, ProverArtifactInfo,
        },
        output,
        prover_task::StatusInfo,
    },
};

const FRONTIER_SOURCE: &str = "local_ee_sled_best_ee_account_state";
const FRONTIER_CAVEAT: &str =
    "This is the last accepted EE account state observed by this EE node; it may lag canonical OL.";

/// Revert EE batches from the provided batch index.
///
/// The command removes batch metadata for `idx >= --from-batch-idx`, deletes
/// all unfinalized exec blocks at or above the first reverted block height,
/// and cleans exact prover rows derivable from the affected batch/chunk ids.
/// Dry-run unless `--force` is passed. `--tx-export` writes raw affected
/// transactions even during dry-run. If the rollback crosses the local
/// accepted EE frontier, the command also rolls back EE account-state rows to
/// the kept batch tip or blocks if that target state is unavailable.
///
/// Forced execution is intentionally not one cross-tree transaction. Batch
/// metadata is reverted last so a failed pre-final mutation can be retried
/// while the command can still reconstruct the affected batch/chunk plan.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-revert-batches")]
pub(crate) struct EeRevertBatchesArgs {
    /// first batch index to revert; batch 0 (genesis) cannot be reverted
    #[argh(option)]
    pub(crate) from_batch_idx: u64,

    /// write affected block transactions to this JSON file for manual rebroadcast
    #[argh(option)]
    pub(crate) tx_export: Option<PathBuf>,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

struct RevertPlan {
    report: EeRevertBatchesReport,
    account_state_rollback: Option<AccountStateRollbackPlan>,
    block_deletions: Vec<BlockDeletionPlan>,
    chunk_artifacts: Vec<ChunkArtifactPlan>,
    batch_artifacts: Vec<BatchArtifactPlan>,
}

#[derive(Debug)]
struct AccountStateRollbackPlan {
    target_epoch: u32,
}

struct BlockDeletionPlan {
    report_index: usize,
    hash: Hash,
    blocknum: u64,
}

struct ChunkArtifactPlan {
    report_index: usize,
    task_key: Vec<u8>,
}

struct BatchArtifactPlan {
    report_index: usize,
    batch_id: BatchId,
    task_key: Vec<u8>,
}

/// Revert EE batches and the corresponding unfinalized exec-block suffix.
pub(crate) async fn ee_revert_batches(
    storage: &EeNodeStorage,
    prover_db: &EeProverDbSled,
    args: EeRevertBatchesArgs,
) -> Result<(), DisplayedError> {
    let mut plan = build_plan(storage, prover_db, &args).await?;

    if let Some(export_path) = &args.tx_export {
        write_tx_export(export_path, &plan.report.affected_blocks)?;
        plan.report.mutation.tx_export_written = true;
    }

    if plan.report.blocked {
        let reason = plan.report.block_reason.clone();
        output(&plan.report, args.output_format)?;
        if args.force {
            return Err(DisplayedError::UserError(
                "EE batch rollback is blocked".to_string(),
                Box::new(reason),
            ));
        }
        return Ok(());
    }

    if !args.force {
        output(&plan.report, args.output_format)?;
        print_force_hint();
        return Ok(());
    }

    // This command mutates multiple sled trees and the prover DB without one
    // outer transaction. Keep batch metadata as the final mutation so a
    // pre-final failure can be retried with the same `--from-batch-idx`.
    if let Some(account_state_rollback) = &plan.account_state_rollback {
        storage
            .rollback_ee_account_state(account_state_rollback.target_epoch)
            .await
            .internal_error("Failed to roll back EE account-state tracker")?;
        plan.report.account_state_rollback.performed = true;
    }

    plan.block_deletions
        .sort_by_cached_key(|deletion| (Reverse(deletion.blocknum), hash_hex(deletion.hash)));
    for deletion in &plan.block_deletions {
        storage
            .delete_exec_block(deletion.hash)
            .await
            .internal_error("Failed to delete EE exec block")?;
        plan.report.affected_blocks[deletion.report_index].deleted = true;
        plan.report.mutation.exec_blocks_deleted += 1;
    }

    for artifact in &plan.chunk_artifacts {
        if prover_db
            .delete_task(artifact.task_key.clone())
            .internal_error("Failed to delete EE chunk prover task")?
        {
            plan.report.affected_chunks[artifact.report_index]
                .task
                .deleted = true;
            plan.report.mutation.chunk_tasks_deleted += 1;
        }

        if prover_db
            .delete_chunk_receipt(&artifact.task_key)
            .internal_error("Failed to delete EE chunk receipt")?
        {
            plan.report.affected_chunks[artifact.report_index].receipt_deleted = true;
            plan.report.mutation.chunk_receipts_deleted += 1;
        }
    }

    for artifact in &plan.batch_artifacts {
        if prover_db
            .delete_task(artifact.task_key.clone())
            .internal_error("Failed to delete EE acct prover task")?
        {
            plan.report.affected_batches[artifact.report_index]
                .acct_task
                .deleted = true;
            plan.report.mutation.acct_tasks_deleted += 1;
        }

        if prover_db
            .delete_acct_proof(artifact.batch_id)
            .internal_error("Failed to delete EE acct proof")?
        {
            plan.report.affected_batches[artifact.report_index].acct_proof_deleted = true;
            plan.report.mutation.acct_proofs_deleted += 1;
        }
    }

    let revert_to_batch_idx = args.from_batch_idx - 1;
    storage
        .revert_batches(revert_to_batch_idx)
        .await
        .internal_error("Failed to revert EE batch metadata")?;
    plan.report.mutation.batch_rows_reverted = plan.report.affected_batches.len();

    plan.report.dry_run = false;
    plan.report.mutation.force = true;
    output(&plan.report, args.output_format)
}

async fn build_plan(
    storage: &EeNodeStorage,
    prover_db: &EeProverDbSled,
    args: &EeRevertBatchesArgs,
) -> Result<RevertPlan, DisplayedError> {
    if args.from_batch_idx == 0 {
        return Err(DisplayedError::UserError(
            "Cannot revert genesis batch; --from-batch-idx must be greater than 0".to_string(),
            Box::new(args.from_batch_idx),
        ));
    }

    let (latest_batch, _) = storage
        .get_latest_batch()
        .await
        .internal_error("Failed to read latest EE batch")?
        .ok_or_else(|| {
            DisplayedError::UserError("No EE batches found".to_string(), Box::new(()))
        })?;
    let latest_batch_idx = latest_batch.idx();
    if args.from_batch_idx > latest_batch_idx {
        return Err(DisplayedError::UserError(
            format!(
                "--from-batch-idx {} is after latest EE batch {latest_batch_idx}",
                args.from_batch_idx
            ),
            Box::new(args.from_batch_idx),
        ));
    }

    let (first_batch, _) = storage
        .get_batch_by_idx(args.from_batch_idx)
        .await
        .internal_error("Failed to read first reverted EE batch")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "EE batch not found for --from-batch-idx".to_string(),
                Box::new(args.from_batch_idx),
            )
        })?;
    let first_reverted_hash = first_batch.blocks_iter().next().ok_or_else(|| {
        DisplayedError::InternalError(
            "Non-genesis batch unexpectedly has no blocks".to_string(),
            Box::new(first_batch.idx()),
        )
    })?;
    let first_reverted_block_height = first_batch_blocknum(&first_batch)?;

    let (kept_batch, _) = storage
        .get_batch_by_idx(args.from_batch_idx - 1)
        .await
        .internal_error("Failed to read kept EE batch before rollback range")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "Kept EE batch before rollback range is missing".to_string(),
                Box::new(args.from_batch_idx - 1),
            )
        })?;

    let mut affected_batches = Vec::new();
    let mut affected_chunks = Vec::new();
    let mut chunk_artifacts = Vec::new();
    let mut batch_artifacts = Vec::new();
    let mut block_to_batch = HashMap::new();
    let mut reverted_batch_blocks = HashSet::new();

    for batch_idx in args.from_batch_idx..=latest_batch_idx {
        let (batch, status) = storage
            .get_batch_by_idx(batch_idx)
            .await
            .internal_error("Failed to read affected EE batch")?
            .ok_or_else(|| {
                DisplayedError::UserError(
                    "Missing EE batch inside rollback range".to_string(),
                    Box::new(batch_idx),
                )
            })?;
        for block_hash in batch.blocks_iter() {
            reverted_batch_blocks.insert(block_hash);
            block_to_batch.insert(block_hash, batch.idx());
        }

        let batch_id = batch.id();
        let chunk_ids = storage
            .get_batch_chunks(batch_id)
            .await
            .internal_error("Failed to read EE batch chunk associations")?
            .unwrap_or_default();

        let acct_task_key = encode_batch_task_key(batch_id);
        let acct_task_record = prover_db
            .get_task(acct_task_key.clone())
            .internal_error("Failed to read EE acct prover task")?;
        let acct_proof_exists = prover_db
            .has_acct_proof(batch_id)
            .internal_error("Failed to read EE acct proof")?;
        let report_index = affected_batches.len();
        affected_batches.push(AffectedBatchInfo {
            idx: batch.idx(),
            id: batch_id.to_string(),
            status: batch_status_name(&status).to_string(),
            update_seq_no: batch.update_seq_no(),
            prev_block: hash_hex(batch.prev_block()),
            last_block: hash_hex(batch.last_block()),
            last_blocknum: batch.last_blocknum(),
            block_count: batch.blocks_iter().count(),
            chunk_count: chunk_ids.len(),
            acct_task: ProverArtifactInfo {
                key_hex: hex::encode(&acct_task_key),
                existed: acct_task_record.is_some(),
                deleted: false,
                status: acct_task_record
                    .as_ref()
                    .map(|record| StatusInfo::from(record.status())),
            },
            acct_proof_exists,
            acct_proof_deleted: false,
        });
        batch_artifacts.push(BatchArtifactPlan {
            report_index,
            batch_id,
            task_key: acct_task_key,
        });

        for chunk_id in chunk_ids {
            let chunk_record = storage
                .get_chunk_by_id(chunk_id)
                .await
                .internal_error("Failed to read affected EE chunk")?;
            let task_key = encode_chunk_task_key(chunk_id);
            let task_record = prover_db
                .get_task(task_key.clone())
                .internal_error("Failed to read EE chunk prover task")?;
            let receipt_exists = prover_db
                .get_chunk_receipt(&task_key)
                .internal_error("Failed to read EE chunk receipt")?
                .is_some();
            let report_index = affected_chunks.len();
            affected_chunks.push(build_affected_chunk_info(
                batch.idx(),
                chunk_id,
                chunk_record,
                &task_key,
                task_record.as_ref(),
                receipt_exists,
            ));
            chunk_artifacts.push(ChunkArtifactPlan {
                report_index,
                task_key,
            });
        }
    }

    let local_accepted_frontier =
        accepted_frontier_info(storage, latest_batch_idx, args.from_batch_idx).await?;
    let account_state_rollback =
        account_state_rollback_info(storage, &local_accepted_frontier, kept_batch.last_block())
            .await?;

    let mut warnings = Vec::new();
    if local_accepted_frontier.crosses_accepted_frontier {
        warnings.push(
            "Rollback crosses the locally observed OL-accepted EE batch frontier; EE account-state tracker will be rolled back too. OL rollback may also be required."
                .to_string(),
        );
    }

    let finalized_height = storage
        .get_finalized_height(first_reverted_hash)
        .await
        .internal_error("Failed to check finalized status for first reverted EE block")?;
    let mut blocked = false;
    let mut block_reason = None;
    if let Some(height) = finalized_height {
        blocked = true;
        block_reason = Some(format!(
            "first reverted block {} is finalized at height {height}",
            hash_hex(first_reverted_hash)
        ));
    }
    let account_state_rollback_plan =
        match build_account_state_rollback_plan(&account_state_rollback, kept_batch.last_block()) {
            Ok(plan) => plan,
            Err(reason) => {
                blocked = true;
                block_reason.get_or_insert(reason);
                None
            }
        };

    let unfinalized_blocks = storage
        .get_unfinalized_blocks()
        .await
        .internal_error("Failed to list unfinalized EE blocks")?;
    let mut affected_blocks = Vec::new();
    let mut block_deletions = Vec::new();
    let mut seen_delete_hashes = HashSet::new();

    for block_hash in unfinalized_blocks {
        let Some(block_record) = storage
            .get_exec_block(block_hash)
            .await
            .internal_error("Failed to read unfinalized EE exec block")?
        else {
            warnings.push(format!(
                "unfinalized block index pointed at missing block {}",
                hash_hex(block_hash)
            ));
            continue;
        };

        if block_record.blocknum() < first_reverted_block_height {
            continue;
        }

        let payload = storage
            .get_block_payload(block_hash)
            .await
            .internal_error("Failed to read EE block payload")?;
        let transactions = match payload {
            Some(payload) => block_transactions(&payload, args.tx_export.is_some())?,
            None => {
                warnings.push(format!(
                    "missing payload for affected EE block {}",
                    hash_hex(block_hash)
                ));
                Vec::new()
            }
        };

        let report_index = affected_blocks.len();
        affected_blocks.push(AffectedBlockInfo {
            blocknum: block_record.blocknum(),
            hash: hash_hex(block_hash),
            parent_hash: hash_hex(block_record.parent_blockhash()),
            batch_idx: block_to_batch.get(&block_hash).copied(),
            in_reverted_batch: reverted_batch_blocks.contains(&block_hash),
            delete_planned: true,
            deleted: false,
            tx_count: transactions.len(),
            transactions,
        });
        seen_delete_hashes.insert(block_hash);
        block_deletions.push(BlockDeletionPlan {
            report_index,
            hash: block_hash,
            blocknum: block_record.blocknum(),
        });
    }

    let mut reverted_batch_blocks_sorted =
        reverted_batch_blocks.iter().copied().collect::<Vec<_>>();
    reverted_batch_blocks_sorted.sort_by_key(|hash| hash_hex(*hash));
    let mut missing_affected_block_count = 0usize;
    let mut first_missing_affected_block = None;
    for block_hash in reverted_batch_blocks_sorted {
        if !seen_delete_hashes.contains(&block_hash) {
            if storage
                .get_finalized_height(block_hash)
                .await
                .internal_error("Failed to check finalized status for affected EE block")?
                .is_some()
            {
                blocked = true;
                block_reason.get_or_insert_with(|| {
                    format!(
                        "affected batch block {} is finalized and cannot be deleted",
                        hash_hex(block_hash)
                    )
                });
            } else {
                missing_affected_block_count += 1;
                first_missing_affected_block.get_or_insert(block_hash);
            }
        }
    }
    if missing_affected_block_count > 0 {
        warnings.push(format!(
            "{} affected batch block(s) were not found in the unfinalized suffix to delete; this is expected when retrying after a partial rollback that already deleted exec blocks. first_missing={}",
            missing_affected_block_count,
            hash_hex(first_missing_affected_block.expect("missing count is non-zero")),
        ));
    }

    affected_blocks.sort_by(|a, b| {
        a.blocknum
            .cmp(&b.blocknum)
            .then_with(|| a.hash.cmp(&b.hash))
    });
    let report_index_by_hash = affected_blocks
        .iter()
        .enumerate()
        .map(|(idx, block)| (block.hash.clone(), idx))
        .collect::<HashMap<_, _>>();
    for deletion in &mut block_deletions {
        if let Some(idx) = report_index_by_hash.get(&hash_hex(deletion.hash)) {
            deletion.report_index = *idx;
        }
    }
    let affected_block_summary = affected_block_summary(&affected_blocks);

    let orphan_notes = vec![
        "Old block witness rows are keyed by old block hash and are left in place; rebuilt blocks get new hashes and fresh witnesses.".to_string(),
        "Chunk rows may remain orphaned after batch metadata is removed; alpen-client startup cleanup handles chunks that no longer belong to a batch.".to_string(),
        "DA/broadcast rows are not removed by this command.".to_string(),
    ];

    Ok(RevertPlan {
        report: EeRevertBatchesReport {
            dry_run: true,
            from_batch_idx: args.from_batch_idx,
            revert_to_batch_idx: args.from_batch_idx - 1,
            latest_batch_idx_before: latest_batch_idx,
            first_reverted_block_height,
            first_reverted_block_hash: hash_hex(first_reverted_hash),
            local_accepted_frontier,
            account_state_rollback,
            warnings,
            blocked,
            block_reason,
            affected_block_summary,
            affected_batches,
            affected_chunks,
            affected_blocks,
            orphan_notes,
            tx_export_path: args
                .tx_export
                .as_ref()
                .map(|path| path.display().to_string()),
            mutation: MutationInfo {
                force: args.force,
                ..MutationInfo::default()
            },
        },
        account_state_rollback: account_state_rollback_plan,
        block_deletions,
        chunk_artifacts,
        batch_artifacts,
    })
}

fn first_batch_blocknum(batch: &Batch) -> Result<u64, DisplayedError> {
    let block_count = batch.blocks_iter().count();
    if block_count == 0 {
        return Err(DisplayedError::InternalError(
            "Non-genesis batch unexpectedly has no blocks".to_string(),
            Box::new(batch.idx()),
        ));
    }

    let block_count = u64::try_from(block_count).map_err(|err| {
        DisplayedError::InternalError(
            "Batch block count does not fit into u64".to_string(),
            Box::new(err),
        )
    })?;
    batch
        .last_blocknum()
        .checked_add(1)
        .and_then(|exclusive_end| exclusive_end.checked_sub(block_count))
        .ok_or_else(|| {
            DisplayedError::InternalError(
                "Batch block count is inconsistent with last block height".to_string(),
                Box::new(batch.idx()),
            )
        })
}

fn build_account_state_rollback_plan(
    account_state_rollback: &AccountStateRollbackInfo,
    kept_batch_tip: Hash,
) -> Result<Option<AccountStateRollbackPlan>, String> {
    if !account_state_rollback.required {
        return Ok(None);
    }

    account_state_rollback
        .target_epoch
        .map(|target_epoch| Some(AccountStateRollbackPlan { target_epoch }))
        .ok_or_else(|| {
            format!(
                "rollback crosses locally accepted frontier, but no EE account-state epoch points at kept batch tip {}",
                hash_hex(kept_batch_tip)
            )
        })
}

fn affected_block_summary(affected_blocks: &[AffectedBlockInfo]) -> AffectedBlockSummary {
    AffectedBlockSummary {
        total_count: affected_blocks.len(),
        reverted_batch: block_range_summary(
            affected_blocks
                .iter()
                .filter(|block| block.in_reverted_batch),
        ),
        unbatched_suffix: block_range_summary(
            affected_blocks
                .iter()
                .filter(|block| !block.in_reverted_batch),
        ),
    }
}

fn block_range_summary<'a>(
    blocks: impl Iterator<Item = &'a AffectedBlockInfo>,
) -> BlockRangeSummary {
    let mut count = 0;
    let mut first_blocknum = None;
    let mut last_blocknum = None;

    for block in blocks {
        count += 1;
        first_blocknum.get_or_insert(block.blocknum);
        last_blocknum = Some(block.blocknum);
    }

    BlockRangeSummary {
        count,
        first_blocknum,
        last_blocknum,
    }
}

async fn accepted_frontier_info(
    storage: &EeNodeStorage,
    latest_batch_idx: u64,
    from_batch_idx: u64,
) -> Result<AcceptedFrontierInfo, DisplayedError> {
    let accepted_tip = storage
        .best_ee_account_state()
        .await
        .internal_error("Failed to read best EE account state")?
        .map(|state| (state.ol_epoch(), state.last_exec_blkid()));
    let best_epoch = accepted_tip.map(|(epoch, _)| epoch);
    let accepted_tip = accepted_tip.map(|(_, tip)| tip);

    let mut accepted_batch_idx = None;
    if let Some(accepted_tip) = accepted_tip {
        // Offline maintenance path: there is no reverse index from EE exec
        // block hash to batch idx, so scan stored batches and stop at the
        // first match.
        for batch_idx in 0..=latest_batch_idx {
            let Some((batch, _)) = storage
                .get_batch_by_idx(batch_idx)
                .await
                .internal_error("Failed to scan EE batches for accepted frontier")?
            else {
                continue;
            };
            if batch.last_block() == accepted_tip {
                accepted_batch_idx = Some(batch.idx());
                break;
            }
        }
    }

    Ok(build_accepted_frontier_info(
        best_epoch,
        accepted_tip,
        accepted_batch_idx,
        from_batch_idx,
    ))
}

fn build_accepted_frontier_info(
    best_epoch: Option<u32>,
    accepted_tip: Option<Hash>,
    accepted_batch_idx: Option<u64>,
    from_batch_idx: u64,
) -> AcceptedFrontierInfo {
    AcceptedFrontierInfo {
        source: FRONTIER_SOURCE,
        last_exec_block_hash: accepted_tip.map(hash_hex),
        best_epoch,
        accepted_batch_idx,
        crosses_accepted_frontier: accepted_batch_idx.is_some_and(|idx| from_batch_idx <= idx),
        caveat: FRONTIER_CAVEAT,
    }
}

async fn account_state_rollback_info(
    storage: &EeNodeStorage,
    accepted_frontier: &AcceptedFrontierInfo,
    target_exec_block: Hash,
) -> Result<AccountStateRollbackInfo, DisplayedError> {
    let required = accepted_frontier.crosses_accepted_frontier;
    let mut info = AccountStateRollbackInfo {
        required,
        target_epoch: None,
        target_exec_block_hash: required.then(|| hash_hex(target_exec_block)),
        performed: false,
    };

    if !required {
        return Ok(info);
    }

    let Some(best_epoch) = accepted_frontier.best_epoch else {
        return Ok(info);
    };

    // There is no reverse index from EE exec block hash to account-state
    // epoch. Walk backward from the observed best epoch and stop at the first
    // state that points at the kept batch tip.
    for epoch in (0..=best_epoch).rev() {
        let state = match storage.ee_account_state(OLBlockOrEpoch::Epoch(epoch)).await {
            Ok(Some(state)) => state,
            Ok(None) | Err(StorageError::StateNotFound(_)) => continue,
            Err(e) => {
                return Err(DisplayedError::InternalError(
                    "Failed to scan EE account-state rollback target".to_string(),
                    Box::new(e),
                ));
            }
        };

        if state.last_exec_blkid() == target_exec_block {
            info.target_epoch = Some(epoch);
            break;
        }
    }

    Ok(info)
}

fn build_affected_chunk_info(
    batch_idx: u64,
    chunk_id: ChunkId,
    chunk_record: Option<(Chunk, ChunkStatus)>,
    task_key: &[u8],
    task_record: Option<&strata_paas::TaskRecordData>,
    receipt_exists: bool,
) -> AffectedChunkInfo {
    let (idx, status, last_blocknum, block_count) = match chunk_record {
        Some((chunk, status)) => (
            Some(chunk.idx()),
            Some(chunk_status_name(&status).to_string()),
            Some(chunk.last_blocknum()),
            Some(chunk.blocks_iter().count()),
        ),
        None => (None, None, None, None),
    };

    AffectedChunkInfo {
        batch_idx,
        idx,
        id: chunk_id_string(chunk_id),
        status,
        prev_block: hash_hex(chunk_id.prev_block()),
        last_block: hash_hex(chunk_id.last_block()),
        last_blocknum,
        block_count,
        task: ProverArtifactInfo {
            key_hex: hex::encode(task_key),
            existed: task_record.is_some(),
            deleted: false,
            status: task_record.map(|record| StatusInfo::from(record.status())),
        },
        receipt_exists,
        receipt_deleted: false,
    }
}

fn block_transactions(
    payload: &ExecBlockPayload,
    include_raw: bool,
) -> Result<Vec<BlockTransactionInfo>, DisplayedError> {
    let payload = AlpenBuiltPayload::from_bytes(payload.as_bytes())
        .internal_error("Failed to decode EE exec block payload")?;
    Ok(payload
        .block()
        .body()
        .transactions()
        .enumerate()
        .map(|(index, tx)| BlockTransactionInfo {
            index,
            hash: format!("{:x}", tx.recalculate_hash()),
            raw_tx_hex: include_raw.then(|| hex::encode(tx.encoded_2718())),
        })
        .collect())
}

fn write_tx_export(
    path: &PathBuf,
    affected_blocks: &[AffectedBlockInfo],
) -> Result<(), DisplayedError> {
    #[derive(serde::Serialize)]
    struct TxExportBlock<'a> {
        blocknum: u64,
        hash: &'a str,
        parent_hash: &'a str,
        batch_idx: Option<u64>,
        transactions: Vec<TxExportTransaction<'a>>,
    }

    #[derive(serde::Serialize)]
    struct TxExportTransaction<'a> {
        index: usize,
        hash: &'a str,
        raw_tx_hex: &'a str,
    }

    let blocks_with_txs = affected_blocks
        .iter()
        .filter(|block| block.tx_count > 0)
        .map(|block| TxExportBlock {
            blocknum: block.blocknum,
            hash: &block.hash,
            parent_hash: &block.parent_hash,
            batch_idx: block.batch_idx,
            transactions: block
                .transactions
                .iter()
                .map(|tx| TxExportTransaction {
                    index: tx.index,
                    hash: &tx.hash,
                    raw_tx_hex: tx.raw_tx_hex.as_deref().unwrap_or_default(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let encoded = serde_json::to_vec_pretty(&blocks_with_txs)
        .internal_error("Failed to serialize EE rollback transaction export")?;
    fs::write(path, encoded).map_err(|e| {
        DisplayedError::UserError(
            format!("Failed to write transaction export to {}", path.display()),
            Box::new(e),
        )
    })
}

fn hash_hex(hash: Hash) -> String {
    format!("{hash:x}")
}

fn chunk_id_string(chunk_id: ChunkId) -> String {
    format!(
        "{}:{}",
        hash_hex(chunk_id.prev_block()),
        hash_hex(chunk_id.last_block())
    )
}

fn batch_status_name(status: &BatchStatus) -> &'static str {
    match status {
        BatchStatus::Genesis => "genesis",
        BatchStatus::Sealed => "sealed",
        BatchStatus::DaPending { .. } => "da_pending",
        BatchStatus::DaComplete { .. } => "da_complete",
        BatchStatus::ProofPending { .. } => "proof_pending",
        BatchStatus::ProofReady { .. } => "proof_ready",
    }
}

fn chunk_status_name(status: &ChunkStatus) -> &'static str {
    match status {
        ChunkStatus::ProvingNotStarted => "proving_not_started",
        ChunkStatus::ProofPending(_) => "proof_pending",
        ChunkStatus::ProofReady(_) => "proof_ready",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hash(byte: u8) -> Hash {
        Hash::from([byte; 32])
    }

    fn rollback_info(required: bool, target_epoch: Option<u32>) -> AccountStateRollbackInfo {
        AccountStateRollbackInfo {
            required,
            target_epoch,
            target_exec_block_hash: required.then(|| hash_hex(test_hash(9))),
            performed: false,
        }
    }

    #[test]
    fn first_batch_blocknum_uses_batch_metadata() {
        let batch = Batch::new(
            7,
            test_hash(1),
            test_hash(4),
            16,
            vec![test_hash(2), test_hash(3)],
        )
        .unwrap();

        assert_eq!(first_batch_blocknum(&batch).unwrap(), 14);
    }

    #[test]
    fn first_batch_blocknum_handles_single_block_batch() {
        let batch = Batch::new(7, test_hash(1), test_hash(2), 16, Vec::new()).unwrap();

        assert_eq!(first_batch_blocknum(&batch).unwrap(), 16);
    }

    #[test]
    fn accepted_frontier_crosses_when_reverting_accepted_batch() {
        let info = build_accepted_frontier_info(Some(134), Some(test_hash(1)), Some(353), 353);

        assert_eq!(info.accepted_batch_idx, Some(353));
        assert_eq!(info.best_epoch, Some(134));
        assert!(info.crosses_accepted_frontier);
    }

    #[test]
    fn accepted_frontier_does_not_cross_when_reverting_after_accepted_batch() {
        let info = build_accepted_frontier_info(Some(134), Some(test_hash(1)), Some(353), 354);

        assert_eq!(info.accepted_batch_idx, Some(353));
        assert!(!info.crosses_accepted_frontier);
    }

    #[test]
    fn account_state_rollback_plan_is_not_required_after_frontier() {
        let plan = build_account_state_rollback_plan(&rollback_info(false, None), test_hash(3))
            .expect("not crossing accepted frontier should not block");

        assert!(plan.is_none());
    }

    #[test]
    fn account_state_rollback_plan_uses_found_target_epoch() {
        let plan = build_account_state_rollback_plan(&rollback_info(true, Some(42)), test_hash(3))
            .expect("found target epoch should allow rollback")
            .expect("crossing accepted frontier should build rollback plan");

        assert_eq!(plan.target_epoch, 42);
    }

    #[test]
    fn account_state_rollback_plan_blocks_without_target_epoch() {
        let err = build_account_state_rollback_plan(&rollback_info(true, None), test_hash(3))
            .expect_err("crossing accepted frontier without target epoch should block");

        assert!(err.contains("rollback crosses locally accepted frontier"));
        assert!(err.contains(&hash_hex(test_hash(3))));
    }
}
