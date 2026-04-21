//! Shared canonical tip resolution.

use strata_db_types::DbResult;
use strata_identifiers::OLBlockCommitment;
use strata_status::StatusChannel;
use strata_storage::NodeStorage;

/// Resolves the current canonical tip.
///
/// Prefers the in-memory [`StatusChannel`] (published by FCM before the new
/// block's status is written to storage), falling back to
/// `storage.ol_block().get_canonical_tip_async()` when the channel has not
/// been initialized yet.
pub(crate) async fn resolve_canonical_tip(
    status_channel: &StatusChannel,
    storage: &NodeStorage,
) -> DbResult<Option<OLBlockCommitment>> {
    if let Some(tip) = status_channel.get_ol_sync_status().map(|s| s.tip) {
        return Ok(Some(tip));
    }
    storage.ol_block().get_canonical_tip_async().await
}
