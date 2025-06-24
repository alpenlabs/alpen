use strata_db::DbError;
use strata_eectl::{
    engine::{ExecEngineCtl, L2BlockRef},
    errors::EngineError,
    messages::ExecPayloadData,
};
use strata_state::header::L2Header;
use strata_storage::NodeStorage;
use thiserror::Error;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub(crate) enum Error {
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
    debug!("Syncing chainstate to EL");
    let l2_block_manager = storage.l2();
    let tip_block = match l2_block_manager.get_tip_block_blocking()? {
        Some(block) => block,
        None => {
            info!("No L2 blocks found, nothing to sync");
            return Ok(()); // nothing to sync
        }
    };
    debug!(%tip_block, "L2 tip block");

    let latest_header = l2_block_manager
        .get_block_data_blocking(&tip_block)?
        .ok_or(DbError::MissingL2Block(tip_block))?
        .header()
        .clone();
    let latest_idx = latest_header.slot();
    info!(%latest_idx, "search for last known idx");

    // last idx of chainstate whose corresponding block is present in el
    let sync_from_idx = find_last_match((0, latest_idx), |idx| {
        let tip_block = l2_block_manager
            .get_blocks_at_height_blocking(idx)?
            .first()
            .cloned()
            .ok_or(DbError::MissingL2BlockHeight(idx))?;
        Ok(engine.check_block_exists(L2BlockRef::Id(tip_block))?)
    })?
    .map(|idx| idx + 1) // sync from next index
    .unwrap_or(0); // sync from genesis
    info!(%sync_from_idx, "last known index in EL");

    // Collect all payloads from sync_from_idx..=latest_idx
    let mut bundles_to_sync = Vec::with_capacity((latest_idx - sync_from_idx) as usize + 1);
    let mut block_to_sync = latest_header.get_blockid();
    for _ in sync_from_idx..=latest_idx {
        let l2block = l2_block_manager
            .get_block_data_blocking(&block_to_sync)?
            .ok_or(DbError::MissingL2Block(block_to_sync))?;
        block_to_sync = *l2block.header().parent();
        bundles_to_sync.push(l2block);
    }
    bundles_to_sync.reverse();

    // Sanity check
    if let (Some(first_bundle), Some(last_bundle)) =
        (bundles_to_sync.first(), bundles_to_sync.last())
    {
        assert_eq!(first_bundle.header().slot(), sync_from_idx);
        assert_eq!(last_bundle.header().slot(), latest_idx);
    }

    for bundle in bundles_to_sync {
        let tip_blockid = bundle.block().header().get_blockid();
        let payload = ExecPayloadData::from_l2_block_bundle(&bundle);
        debug!(?payload, "Submitting payload to engine");
        engine.submit_payload(payload)?;
        engine.update_safe_block(tip_blockid)?;
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
