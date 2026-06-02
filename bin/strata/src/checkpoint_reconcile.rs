//! Reconciles local checkpoint artifacts against ASM-accepted state.

use anyhow::{Context, Result};
use strata_asm_common::Subprotocol;
use strata_asm_proto_checkpoint::CheckpointSubprotocol;
use strata_checkpoint_types::CheckpointProofTask;
use strata_identifiers::{Epoch, EpochCommitment};
use strata_node_context::NodeContext;
use tracing::{debug, info};

/// Deletes local checkpoint artifacts after ASM's accepted checkpoint tip.
///
/// Checkpoint payloads, proofs, and prover tasks past the ASM verified tip are
/// local candidate state. Rebuilding them on startup prevents a rotated OL
/// image from reusing stale pre-rotation proof artifacts.
pub(crate) fn reconcile_unaccepted_checkpoint_artifacts(nodectx: &NodeContext) -> Result<()> {
    if nodectx.config().prover.is_none() {
        return Ok(());
    }

    let Some(first_unaccepted_epoch) = first_unaccepted_checkpoint_epoch(nodectx)? else {
        return Ok(());
    };

    let storage = nodectx.storage();
    let mut cleanup_commitments =
        checkpoint_commitments_from_epoch(nodectx, first_unaccepted_epoch)?;

    let deleted_payloads = storage
        .ol_checkpoint()
        .del_checkpoint_payload_entries_from_epoch_blocking(first_unaccepted_epoch)
        .context("delete unaccepted checkpoint payloads")?;
    extend_missing(&mut cleanup_commitments, deleted_payloads.iter().copied());

    let mut deleted_proofs = 0usize;
    let mut deleted_tasks = 0usize;

    for commitment in cleanup_commitments {
        if storage
            .checkpoint_proof()
            .del_proof(commitment)
            .with_context(|| format!("delete checkpoint proof for commitment {commitment}"))?
        {
            deleted_proofs += 1;
        }

        let task_key = CheckpointProofTask(commitment).to_key_bytes();
        if storage
            .prover_tasks()
            .delete_task(&task_key)
            .with_context(|| format!("delete checkpoint prover task for commitment {commitment}"))?
        {
            deleted_tasks += 1;
        }
    }

    if !deleted_payloads.is_empty() || deleted_proofs > 0 || deleted_tasks > 0 {
        info!(
            first_unaccepted_epoch,
            deleted_payloads = deleted_payloads.len(),
            deleted_proofs,
            deleted_tasks,
            "reconciled unaccepted checkpoint artifacts against ASM verified tip"
        );
    }

    Ok(())
}

fn checkpoint_commitments_from_epoch(
    nodectx: &NodeContext,
    first_unaccepted_epoch: Epoch,
) -> Result<Vec<EpochCommitment>> {
    let storage = nodectx.storage();
    let Some(last_summarized_epoch) = storage
        .ol_checkpoint()
        .get_last_summarized_epoch_blocking()
        .context("read last summarized checkpoint epoch")?
    else {
        return Ok(Vec::new());
    };

    if first_unaccepted_epoch > last_summarized_epoch {
        return Ok(Vec::new());
    }

    let mut commitments = Vec::new();
    for epoch in first_unaccepted_epoch..=last_summarized_epoch {
        let epoch_commitments = storage
            .ol_checkpoint()
            .get_epoch_commitments_at_blocking(epoch)
            .with_context(|| format!("read checkpoint commitments for epoch {epoch}"))?;
        extend_missing(&mut commitments, epoch_commitments);
    }

    Ok(commitments)
}

fn extend_missing<T>(items: &mut Vec<T>, candidates: impl IntoIterator<Item = T>)
where
    T: Copy + Eq,
{
    for candidate in candidates {
        if !items.contains(&candidate) {
            items.push(candidate);
        }
    }
}

fn first_unaccepted_checkpoint_epoch(nodectx: &NodeContext) -> Result<Option<Epoch>> {
    let Some((asm_l1, asm_state)) = nodectx
        .storage()
        .asm()
        .fetch_most_recent_state()
        .context("fetch latest ASM state")?
    else {
        debug!("latest ASM state is not available; skipping checkpoint artifact reconciliation");
        return Ok(None);
    };

    let checkpoint_state = asm_state
        .state()
        .find_section(<CheckpointSubprotocol as Subprotocol>::ID)
        .context("latest ASM state is missing checkpoint subprotocol state")?
        .try_to_state::<CheckpointSubprotocol>()
        .context("decode checkpoint subprotocol state")?;

    let verified_epoch = checkpoint_state.verified_tip().epoch;
    let Some(first_unaccepted_epoch) = verified_epoch.checked_add(1) else {
        debug!(
            %asm_l1,
            verified_epoch,
            "ASM checkpoint verified tip is at maximum epoch; no checkpoint artifacts to reconcile"
        );
        return Ok(None);
    };

    debug!(
        %asm_l1,
        verified_epoch,
        first_unaccepted_epoch,
        "resolved first unaccepted checkpoint epoch from ASM verified tip"
    );

    Ok(Some(first_unaccepted_epoch))
}
