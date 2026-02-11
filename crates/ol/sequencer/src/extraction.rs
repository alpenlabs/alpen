use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_db_types::types::OLCheckpointStatus;
use strata_primitives::OLBlockId;
use strata_storage::NodeStorage;

use crate::{BlockSigningDuty, CheckpointSigningDuty, Duty, Error, TemplateManager};

/// Extract sequencer duties
pub async fn extract_duties(
    template_mgr: &TemplateManager,
    tip_blkid: OLBlockId,
    node_storage: &NodeStorage,
) -> Result<Vec<Duty>, Error> {
    let mut duties = vec![];

    // Block duties. Just get the block template from manager.
    let template = template_mgr.generate_template(tip_blkid).await?;
    let blkduty = BlockSigningDuty::new(template);
    duties.push(Duty::SignBlock(blkduty));

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
