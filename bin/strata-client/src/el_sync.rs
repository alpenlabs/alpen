use strata_db_types::DbError;
use strata_eectl::{
    engine::{ExecEngineCtl, L2BlockRef},
    errors::EngineError,
    messages::ExecPayloadData,
};
use strata_ol_chain_types::L2BlockId;
use strata_storage::NodeStorage;
use thiserror::Error;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("missing write batch for l2block {0}")]
    MissingWriteBatch(L2BlockId),
    #[error("missing l2block {0}")]
    MissingL2Block(L2BlockId),
    #[error("db: {0}")]
    Db(#[from] DbError),
    #[error("engine: {0}")]
    Engine(#[from] EngineError),
}

/// Sync missing blocks in EL using payloads stored in L2 block database.
///
/// TODO: retry on network errors
pub(crate) fn sync_chainstate_to_el(
    storage: &NodeStorage,
    engine: &impl ExecEngineCtl,
) -> Result<(), Error> {
    let chainstate_manager = storage.chainstate();
    let l2_block_manager = storage.l2();

    // Get the tip block - represents the canonical chain
    let tip_blockid = l2_block_manager.get_tip_block_blocking()?;

    // Get the chainstate for the tip to find the latest slot
    let tip_wb = chainstate_manager
        .get_slot_write_batch_blocking(tip_blockid)?
        .ok_or(Error::MissingWriteBatch(tip_blockid))?;
    let latest_slot = tip_wb.new_toplevel_state().chain_tip_slot();

    // Build canonical chain by collecting block IDs for slots with chainstate
    // Chainstate tracks the canonical chain, so we only get canonical blocks
    let mut canonical_chain = Vec::new();
    for slot in 0..=latest_slot {
        // Try to get blocks at this height in the canonical chain
        let blocks_at_slot = l2_block_manager.get_blocks_at_height_blocking(slot)?;

        // Find which block (if any) at this slot is in the canonical chain
        // by checking if it has a chainstate write batch
        for blkid in blocks_at_slot {
            if chainstate_manager
                .get_slot_write_batch_blocking(blkid)?
                .is_some()
            {
                canonical_chain.push(blkid);
                break; // Only one canonical block per slot
            }
        }
    }

    let earliest_idx = 0;
    let latest_idx = canonical_chain.len().saturating_sub(1);

    info!(total_blocks = %canonical_chain.len(), "searching for last known block in EL");

    // Find the last block in the canonical chain that exists in EL
    let sync_from_idx = find_last_match((earliest_idx, latest_idx), |idx| {
        let blkid = canonical_chain[idx];
        Ok(engine.check_block_exists(L2BlockRef::Id(blkid))?)
    })?
    .map(|idx| idx + 1) // sync from next block
    .unwrap_or(0); // sync from genesis

    info!(%sync_from_idx, total_blocks = %canonical_chain.len(), "last known block in EL");

    // Sync all blocks from sync_from_idx onwards
    for (idx, &tip_blockid) in canonical_chain.iter().enumerate().skip(sync_from_idx) {
        debug!(?idx, ?tip_blockid, "Syncing block");

        let Some(l2block) = l2_block_manager.get_block_data_blocking(&tip_blockid)? else {
            return Err(Error::MissingL2Block(tip_blockid));
        };

        let payload = ExecPayloadData::from_l2_block_bundle(&l2block);

        engine.submit_payload(payload)?;
        engine.update_safe_block(tip_blockid)?;
    }

    Ok(())
}

fn find_last_match(
    range: (usize, usize),
    predicate: impl Fn(usize) -> Result<bool, Error>,
) -> Result<Option<usize>, Error> {
    let (mut left, mut right) = range;

    // Handle empty range
    if left > right {
        return Ok(None);
    }

    // Check the leftmost value first
    if !predicate(left)? {
        return Ok(None); // If the leftmost value is false, no values can be true
    }

    let mut best_match = None;

    // Proceed with binary search
    while left <= right {
        let mid = left + (right - left) / 2;

        if predicate(mid)? {
            best_match = Some(mid); // Update best match
            left = mid + 1; // Continue searching in the right half
        } else {
            if mid == 0 {
                break;
            }
            right = mid - 1; // Search in the left half
        }
    }

    Ok(best_match)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_last_match() {
        // find match
        assert!(matches!(
            find_last_match((0, 5), |idx| Ok(idx < 3)),
            Ok(Some(2))
        ));
        // found no match
        assert!(matches!(find_last_match((0, 5), |_| Ok(false)), Ok(None)));
        // got error
        let error_message = "intentional error for test";
        assert!(matches!(
            find_last_match((0, 5), |_| Err(EngineError::Other(error_message.into()))?),
            Err(err) if err.to_string().contains(error_message)
        ));
    }
}
