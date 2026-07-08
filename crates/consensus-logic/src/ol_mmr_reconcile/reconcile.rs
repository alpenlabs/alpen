use strata_acct_types::Mmr64;
use strata_db_types::{MmrId, RawMmrId};
use strata_ol_mmr_index::{
    build_mmr_index_reconcile_plan, MmrIndexEntry, MmrIndexReconcilePlan, MmrIndexReconcileReport,
    MmrIndexTruncation,
};
use strata_ol_state_support_types::MemoryStateBaseLayer;
use tracing::info;

use super::{
    context::OLMmrReconcileCtx,
    error::{OLMmrReconcileError, OLMmrReconcileResult},
    target::OLMmrReconcileTarget,
};

/// Reconciles the MMR index DB against an already-resolved OL state target.
pub async fn reconcile_ol_mmr_index_to_target(
    ctx: &impl OLMmrReconcileCtx,
    target: OLMmrReconcileTarget,
) -> OLMmrReconcileResult<MmrIndexReconcileReport> {
    // Startup reconciliation runs before the chain worker's own idempotent
    // prefill path, so it must seed the genesis sentinel before classification.
    ctx.prefill_l1_block_refs_mmr().await?;

    let target_snark_accounts = target.state.iter_snark_account_ids();
    let target_state_accessor = MemoryStateBaseLayer::new(target.state.as_ref().clone());
    let entries = get_mmr_index_entries(ctx).await?;
    let plan =
        build_mmr_index_reconcile_plan(&target_state_accessor, entries, target_snark_accounts)?;
    let report = execute_mmr_reconcile_plan(ctx, &plan).await?;

    // MMR and OL state indexing are both derived from the selected OL state
    // target. Reconcile indexing after MMR truncation so account queries cannot
    // observe rows that no longer have index leaves.
    ctx.reconcile_ol_state_indexing_to_target(&target).await?;

    if report.indexes_truncated > 0 {
        log_reconcile_report(&target, &report);
    }

    Ok(report)
}

/// Reads every persisted MMR namespace and its current state.
///
/// An undecodable namespace id fails reconciliation as corruption. Skipping it
/// could leave an OL-owned index divergent while startup reports success.
async fn get_mmr_index_entries(
    ctx: &impl OLMmrReconcileCtx,
) -> OLMmrReconcileResult<Vec<MmrIndexEntry>> {
    let mut entries = Vec::new();

    for raw_mmr_id in ctx.list_mmr_ids().await? {
        let mmr_id = decode_mmr_id(&raw_mmr_id)?;
        let leaf_count = ctx.get_mmr_leaf_count(&mmr_id).await?;
        let state = ctx.get_mmr_state_at(&mmr_id, leaf_count).await?;

        entries.push(MmrIndexEntry::new(mmr_id, state));
    }

    Ok(entries)
}

/// Applies a validated reconciliation plan to the MMR index.
///
/// The function validates prefixes before the first write, then truncates each
/// ahead namespace and verifies the final count and state as a regression guard.
async fn execute_mmr_reconcile_plan(
    ctx: &impl OLMmrReconcileCtx,
    plan: &MmrIndexReconcilePlan,
) -> OLMmrReconcileResult<MmrIndexReconcileReport> {
    validate_mmr_reconcile_prefixes(ctx, plan).await?;

    for truncation in plan.truncations() {
        let target = truncation.target();
        let mmr_id = truncation.mmr_id();

        ctx.truncate_mmr_to_leaf_count(mmr_id, target.num_entries())
            .await?;

        let final_leaf_count = ctx.get_mmr_leaf_count(mmr_id).await?;
        validate_final_mmr_leaf_count(truncation, final_leaf_count)?;

        let final_state = ctx.get_mmr_state_at(mmr_id, target.num_entries()).await?;
        validate_final_mmr_state(truncation, &final_state)?;
    }

    Ok(plan.to_report())
}

fn log_reconcile_report(target: &OLMmrReconcileTarget, report: &MmrIndexReconcileReport) {
    info!(
        target = %target.block,
        target_epoch = target.epoch,
        inspected = report.inspected,
        asm_owned_skipped = report.asm_owned_skipped,
        indexes_truncated = report.indexes_truncated,
        leaves_removed = report.leaves_removed,
        "reconciled OL MMR index against persisted OL state"
    );
}

/// Decodes a raw namespace key into a typed [`MmrId`].
fn decode_mmr_id(raw_mmr_id: &RawMmrId) -> OLMmrReconcileResult<MmrId> {
    MmrId::from_bytes(raw_mmr_id).map_err(|source| OLMmrReconcileError::InvalidRawMmrId {
        raw_mmr_id: hex::encode(raw_mmr_id),
        source,
    })
}

fn validate_final_mmr_leaf_count(
    truncation: &MmrIndexTruncation,
    final_leaf_count: u64,
) -> OLMmrReconcileResult<()> {
    let mmr_id = truncation.mmr_id();
    let target = truncation.target();
    if final_leaf_count != target.num_entries() {
        return Err(OLMmrReconcileError::PostTruncateLeafCountMismatch {
            mmr_id: mmr_id.clone(),
            target_leaf_count: target.num_entries(),
            final_leaf_count,
        });
    }

    Ok(())
}

fn validate_final_mmr_state(
    truncation: &MmrIndexTruncation,
    final_state: &Mmr64,
) -> OLMmrReconcileResult<()> {
    let mmr_id = truncation.mmr_id();
    let target = truncation.target();
    if final_state != target {
        return Err(OLMmrReconcileError::PostTruncateStateMismatch {
            mmr_id: mmr_id.clone(),
            leaf_count: target.num_entries(),
        });
    }

    Ok(())
}

/// Verifies that every ahead index has the target state as a prefix.
///
/// This preflight must run before destructive truncation. An index can have a
/// larger leaf count while not being a prefix of the target, which indicates
/// corruption or sibling-chain data rather than stale suffix leaves.
async fn validate_mmr_reconcile_prefixes(
    ctx: &impl OLMmrReconcileCtx,
    plan: &MmrIndexReconcilePlan,
) -> OLMmrReconcileResult<()> {
    for truncation in plan.truncations() {
        let target = truncation.target();
        let mmr_id = truncation.mmr_id();
        let index_prefix_state = ctx.get_mmr_state_at(mmr_id, target.num_entries()).await?;
        if &index_prefix_state != target {
            return Err(OLMmrReconcileError::TargetPrefixNotInIndex {
                mmr_id: mmr_id.clone(),
                target_leaf_count: target.num_entries(),
            });
        }
    }

    Ok(())
}
