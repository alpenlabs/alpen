use strata_state::header::L2Header;
use tracing::{debug, info};

use crate::{
    engine::{ExecEngineCtl, L2BlockRef},
    errors::EngineError,
    messages::ExecPayloadData,
    worker::ExecWorkerContext,
};

/// Sync missing blocks in EL using payloads stored in L2 block database.
///
/// TODO: retry on network errors
pub(crate) fn sync_chainstate_to_el(
    context: &impl ExecWorkerContext,
    engine: &impl ExecEngineCtl,
) -> Result<(), EngineError> {
    info!("Syncing chainstate to EL");
    let tip_block = context.get_cur_tip()?;
    debug!(?tip_block, "L2 tip block");

    let tip_idx = tip_block.slot();

    // last idx of chainstate whose corresponding block is present in el
    let sync_from_idx = find_last_match((0, tip_idx), |idx| {
        let blkid = context.get_blkid_at_height(idx)?;
        engine.check_block_exists(L2BlockRef::Id(blkid))
    })?
    .map(|idx| idx + 1) // sync from next index
    .unwrap_or(0); // sync from genesis
    info!(%sync_from_idx, "last known EL block index");

    if sync_from_idx >= tip_idx {
        info!("EL in sync with chainstate");
        return Ok(());
    }

    // Collect all payloads from sync_from_idx..=tip_idx
    let mut bundles_to_sync = Vec::with_capacity((tip_idx - sync_from_idx) as usize + 1);
    let mut block_to_sync = *tip_block.blkid();
    for _ in sync_from_idx..=tip_idx {
        let l2block = context.get_block(block_to_sync)?;
        block_to_sync = *l2block.header().parent();
        bundles_to_sync.push(l2block);
    }
    bundles_to_sync.reverse();

    // Sanity check
    if let (Some(first_bundle), Some(last_bundle)) =
        (bundles_to_sync.first(), bundles_to_sync.last())
    {
        assert_eq!(first_bundle.header().slot(), sync_from_idx);
        assert_eq!(last_bundle.header().slot(), tip_idx);
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
    predicate: impl Fn(u64) -> Result<bool, EngineError>,
) -> Result<Option<u64>, EngineError> {
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
