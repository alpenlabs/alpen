use strata_asm_proto_checkpoint_types::CheckpointPayload;
use strata_db_types::types::L1BundleStatus;
use strata_ol_block_assembly::{BlockAssemblyError, BlockasmHandle};
use strata_primitives::OLBlockId;
use strata_storage::NodeStorage;
use tracing::debug;

use crate::{BlockSigningDuty, CheckpointSigningDuty, Duty, Error, RevealTxSigningDuty};

/// Extract sequencer duties.
pub async fn extract_duties(
    blockasm: &BlockasmHandle,
    tip_blkid: OLBlockId,
    node_storage: &NodeStorage,
) -> Result<Vec<Duty>, Error> {
    let mut duties = vec![];

    // Block duties. Read-only lookup; generation is handled by GenerationTick.
    match blockasm.get_block_template(tip_blkid).await {
        Ok(template) => {
            let blkduty = BlockSigningDuty::new(template);
            duties.push(Duty::SignBlock(blkduty));
        }
        Err(BlockAssemblyError::NoPendingTemplateForParent(_)) => {
            debug!(
                tip_blkid = ?tip_blkid,
                "no cached template for tip parent; skipping block duty"
            );
        }
        Err(err) => return Err(err.into()),
    }

    // Checkpoint duties
    let unsigned_checkpoint = get_earliest_unsigned_checkpoint(node_storage).await?;
    duties.extend(
        unsigned_checkpoint
            .into_iter()
            .map(CheckpointSigningDuty::new)
            .map(Duty::SignCheckpoint),
    );

    // Payload signing duties
    let pending_payloads = get_pending_payload_duties(node_storage).await?;
    duties.extend(pending_payloads.into_iter().map(Duty::SignRevealTx));

    Ok(duties)
}

/// Gets the earliest unsigned checkpoint
async fn get_earliest_unsigned_checkpoint(
    node_storage: &NodeStorage,
) -> Result<Option<CheckpointPayload>, Error> {
    let ckptdb = node_storage.ol_checkpoint();
    let Some(unsigned_epoch) = ckptdb.get_next_unsigned_checkpoint_epoch_async().await? else {
        return Ok(None);
    };

    let Some(commitment) = ckptdb
        .get_canonical_epoch_commitment_at_async(unsigned_epoch)
        .await?
    else {
        return Ok(None);
    };

    if ckptdb
        .get_checkpoint_signing_entry_async(commitment)
        .await?
        .is_some()
    {
        return Ok(None);
    }

    ckptdb
        .get_checkpoint_payload_entry_async(commitment)
        .await
        .map_err(Into::into)
}

/// Gets payload entries pending an external signature.
async fn get_pending_payload_duties(
    node_storage: &NodeStorage,
) -> Result<Vec<RevealTxSigningDuty>, Error> {
    let l1_writer = node_storage.l1_writer();

    let mut idx = l1_writer.get_next_payload_idx_async().await?;
    let mut duties = vec![];

    while idx > 0 {
        idx -= 1;
        let Some(entry) = l1_writer.get_payload_entry_by_idx_async(idx).await? else {
            break;
        };
        if entry.status == L1BundleStatus::Finalized {
            break;
        }
        if let L1BundleStatus::PendingRevealTxSign(sighash) = entry.status {
            if entry.payload_signature.is_none() {
                duties.push(RevealTxSigningDuty::new(idx, sighash));
            }
        }
    }

    Ok(duties)
}
