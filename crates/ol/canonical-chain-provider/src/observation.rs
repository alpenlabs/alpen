use strata_db_types::{DbError, DbResult};
use strata_identifiers::{Epoch, EpochCommitment};
use strata_storage::{L1BlockManager, OLCheckpointManager};

/// Resolves the unique observed epoch commitment whose L1 reference is canonical.
///
/// Several observations exist for an epoch only after an L1 reorg. Returns `None` if no
/// observation survives. Missing references and orphaned L1 blocks are skipped; multiple
/// survivors are an error. This does not check finality. ASM accepts only one checkpoint
/// per epoch per chain, so two commitments cannot both be canonical on the same L1 chain.
pub fn resolve_observed_epoch_blocking(
    checkpoints: &OLCheckpointManager,
    l1: &L1BlockManager,
    epoch: Epoch,
) -> DbResult<Option<EpochCommitment>> {
    let mut selected = None;
    for candidate in checkpoints.get_observed_checkpoint_commitments_for_epoch_blocking(epoch)? {
        let Some(reference) = checkpoints.get_checkpoint_l1_ref_blocking(candidate)? else {
            continue;
        };
        let block = reference.l1_commitment;
        if l1.get_canonical_blockid_at_height(block.height())? != Some(*block.blkid()) {
            continue;
        }
        if selected.replace(candidate).is_some() {
            return Err(DbError::OLCheckpointHistoryAmbiguous { epoch });
        }
    }
    Ok(selected)
}
