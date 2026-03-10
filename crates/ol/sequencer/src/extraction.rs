use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_db_types::types::OLCheckpointStatus;
use strata_ol_block_assembly::{BlockasmHandle, FullBlockTemplate};
use strata_primitives::OLBlockId;
use strata_storage::NodeStorage;

use crate::{BlockSigningDuty, CheckpointSigningDuty, Duty, Error};

/// Extract sequencer duties
pub async fn extract_duties(
    blockasm: &BlockasmHandle,
    tip_blkid: OLBlockId,
    node_storage: &NodeStorage,
) -> Result<Vec<Duty>, Error> {
    let mut duties = vec![];

    // Block duties. Try to get a cached template, or generate a new one.
    let template = generate_or_get_template(blockasm, tip_blkid).await?;
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

/// Generates a block template or retrieves it from the block-assembly cache.
async fn generate_or_get_template(
    blockasm: &BlockasmHandle,
    parent_block_id: OLBlockId,
) -> Result<FullBlockTemplate, Error> {
    Ok(blockasm
        .get_or_generate_block_template(parent_block_id)
        .await?)
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
