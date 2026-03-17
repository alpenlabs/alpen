use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_db_types::types::OLCheckpointStatus;
use strata_ol_block_assembly::{BlockAssemblyError, BlockasmHandle};
use strata_primitives::OLBlockId;
use strata_storage::NodeStorage;
use tracing::debug;

use crate::{BlockSigningDuty, CheckpointSigningDuty, Duty, Error};

/// Extract sequencer duties
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
    Ok(duties)
}

/// Gets the earliest unsigned checkpoint
async fn get_earliest_unsigned_checkpoint(
    node_storage: &NodeStorage,
) -> Result<Option<CheckpointPayload>, Error> {
    let ckptdb = node_storage.ol_checkpoint();
    let mut unsigned_ckpt = None;

    let Some(mut last_ckpt) = ckptdb.get_last_checkpoint_epoch_async().await? else {
        return Ok(unsigned_ckpt);
    };

    // loop backwards from latest to get the earliest unsigned checkpoint
    loop {
        let Some(ckpt) = ckptdb.get_checkpoint_async(last_ckpt).await? else {
            break;
        };
        if ckpt.status == OLCheckpointStatus::Unsigned {
            unsigned_ckpt = Some(ckpt.checkpoint.clone());
        } else {
            // All the previous checkpoints should be signed already because we sign them in
            // sequence
            break;
        };

        if last_ckpt == 0 {
            break;
        }

        last_ckpt -= 1;
    }
    Ok(unsigned_ckpt)
}
