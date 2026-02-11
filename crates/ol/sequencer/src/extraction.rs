use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_db_types::types::OLCheckpointStatus;
use strata_ol_block_assembly::{BlockAssemblyError, BlockasmHandle, FullBlockTemplate};
use strata_primitives::{OLBlockCommitment, OLBlockId};
use strata_storage::NodeStorage;

use crate::{BlockGenerationConfig, BlockSigningDuty, CheckpointSigningDuty, Duty, Error};

/// Extract sequencer duties
pub async fn extract_duties(
    blockasm: &BlockasmHandle,
    tip_blkid: OLBlockId,
    node_storage: &NodeStorage,
) -> Result<Vec<Duty>, Error> {
    let mut duties = vec![];

    // Block duties. Try to get a cached template, or generate a new one.
    let template = generate_or_get_template(blockasm, node_storage, tip_blkid).await?;
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
    storage: &NodeStorage,
    parent_block_id: OLBlockId,
) -> Result<FullBlockTemplate, Error> {
    // Try to get from block-assembly cache first.
    match blockasm.get_block_template(parent_block_id).await {
        Ok(template) => return Ok(template),
        Err(BlockAssemblyError::NoPendingTemplateForParent(_)) => {
            // Not cached, fall through to generate.
        }
        Err(e) => return Err(e.into()),
    }

    // Fetch parent block to get its slot.
    let parent_block = storage
        .ol_block()
        .get_block_data_async(parent_block_id)
        .await
        .map_err(Error::Database)?
        .ok_or(Error::UnknownBlock(parent_block_id))?;

    let parent_slot = parent_block.header().slot();
    let config = BlockGenerationConfig::new(OLBlockCommitment::new(parent_slot, parent_block_id));

    Ok(blockasm.generate_block_template(config).await?)
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
