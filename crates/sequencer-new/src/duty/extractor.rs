//! Extracts new duties for sequencer for a given consensus state.

use strata_identifiers::OLBlockId;
use strata_params::Params;
use strata_storage::{OLBlockManager, OLCheckpointManager};

use super::{
    errors::Error,
    types::{BlockSigningDuty, CheckpointDuty, Duty},
};

/// Extracts new duties given a current tip and params.
pub async fn extract_duties(
    tip_blkid: OLBlockId,
    ol_block_manager: &OLBlockManager,
    ol_checkpoint_manager: &OLCheckpointManager,
    params: &Params,
) -> Result<Vec<Duty>, Error> {
    let mut duties = vec![];
    duties.extend(extract_block_duties(tip_blkid, ol_block_manager, params).await?);
    duties.extend(extract_batch_duties(ol_checkpoint_manager).await?);
    Ok(duties)
}

async fn extract_block_duties(
    tip_blkid: OLBlockId,
    ol_block_manager: &OLBlockManager,
    params: &Params,
) -> Result<Vec<Duty>, Error> {
    let tip_block_header = ol_block_manager
        .get_block_data_async(tip_blkid)
        .await?
        .ok_or(Error::MissingOLBlock(tip_blkid))?
        .header()
        .clone();
    let tip_block_ts = tip_block_header.timestamp();
    let new_tip_slot = tip_block_header.slot() + 1;

    let target_ts = tip_block_ts + params.rollup().block_time;

    Ok(vec![Duty::SignBlock(BlockSigningDuty::new_simple(
        new_tip_slot,
        tip_blkid,
        target_ts,
    ))])
}

async fn extract_batch_duties(
    ol_checkpoint_manager: &OLCheckpointManager,
) -> Result<Vec<Duty>, Error> {
    let Some(epoch) = ol_checkpoint_manager
        .get_next_unsigned_checkpoint_epoch_async()
        .await?
    else {
        return Ok(Vec::new());
    };

    let Some(entry) = ol_checkpoint_manager.get_checkpoint_async(epoch).await? else {
        return Ok(Vec::new());
    };

    Ok(vec![Duty::CommitBatch(CheckpointDuty::new(
        entry.checkpoint,
    ))])
}
