use strata_db::{traits::BlockStatus, DbError};
use strata_eectl::{
    engine::{ExecEngineCtl, L2BlockRef},
    errors::EngineError,
    messages::ExecPayloadData,
};
use strata_state::id::L2BlockId;
use strata_storage::NodeStorage;
use thiserror::Error;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum Error {
    #[error("missing chainstate for slot {0}")]
    MissingChainstate(u64),
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
pub fn sync_chainstate_to_el(
    storage: &NodeStorage,
    engine: &impl ExecEngineCtl,
) -> Result<(), Error> {
    let chainstate_manager = storage.chainstate();
    let l2_block_manager = storage.l2();
    let earliest_idx = chainstate_manager.get_earliest_write_idx_blocking()?;
    let latest_idx = chainstate_manager.get_last_write_idx_blocking()?;

    info!(%earliest_idx, %latest_idx, "search for last known idx");

    // last idx of chainstate whose corresponding block is present in el
    let sync_from_idx = find_last_match((earliest_idx, latest_idx), |idx| {
        let Some(entry) = chainstate_manager.get_toplevel_chainstate_blocking(idx)? else {
            return Err(Error::MissingChainstate(idx));
        };

        Ok(engine.check_block_exists(L2BlockRef::Id(*entry.tip_blockid()))?)
    })?
    .map(|idx| idx + 1) // sync from next index
    .unwrap_or(0); // sync from genesis

    info!(%sync_from_idx, "last known index in EL");

    for idx in sync_from_idx..=latest_idx {
        debug!(?idx, "Syncing chainstate");
        let Some(chainstate_entry) = chainstate_manager.get_toplevel_chainstate_blocking(idx)?
        else {
            return Err(Error::MissingChainstate(idx));
        };

        let tip_blockid = chainstate_entry.tip_blockid();

        let Some(l2block) = l2_block_manager.get_block_data_blocking(tip_blockid)? else {
            return Err(Error::MissingL2Block(*tip_blockid));
        };

        let payload = ExecPayloadData::from_l2_block_bundle(&l2block);

        engine.submit_payload(payload)?;
        engine.update_safe_block(*tip_blockid)?;
    }

    Ok(())
}

/// Sync missing or invalid L2 block status using chainstate.
///
/// TODO: retry on network errors
pub fn sync_chainstate_l2_status(storage: &NodeStorage) -> Result<(), Error> {
    let chainstate_manager = storage.chainstate();
    let l2_block_manager = storage.l2();
    let earliest_idx = chainstate_manager.get_earliest_write_idx_blocking()?;
    let latest_idx = chainstate_manager.get_last_write_idx_blocking()?;

    info!(%earliest_idx, %latest_idx, "search for last known idx");

    // last idx of chainstate whose corresponding block is present in el
    let sync_from_idx = find_last_match((earliest_idx, latest_idx), |idx| {
        let Some(entry) = chainstate_manager.get_toplevel_chainstate_blocking(idx)? else {
            return Err(Error::MissingChainstate(idx));
        };

        match l2_block_manager.get_block_status_blocking(entry.tip_blockid())? {
            Some(BlockStatus::Valid) => Ok(true),
            _ => Ok(false),
        }
    })?
    .map(|idx| idx + 1) // sync from next index
    .unwrap_or(0); // sync from genesis

    info!(%sync_from_idx, "last valid status");

    for idx in sync_from_idx..=latest_idx {
        debug!(?idx, "Syncing chainstate to L2 status");
        let Some(chainstate_entry) = chainstate_manager.get_toplevel_chainstate_blocking(idx)?
        else {
            return Err(Error::MissingChainstate(idx));
        };

        let tip_blockid = chainstate_entry.tip_blockid();

        if l2_block_manager
            .get_block_data_blocking(tip_blockid)?
            .is_none()
        {
            return Err(Error::MissingL2Block(*tip_blockid));
        };

        l2_block_manager.set_block_status_blocking(tip_blockid, BlockStatus::Valid)?;
    }

    Ok(())
}

fn find_last_match(
    range: (u64, u64),
    predicate: impl Fn(u64) -> Result<bool, Error>,
) -> Result<Option<u64>, Error> {
    let (mut left, mut right) = range;

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
