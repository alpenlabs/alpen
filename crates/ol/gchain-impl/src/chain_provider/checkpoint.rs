//! Checkpoint links in the chain graph.

use strata_gchain_types::{LinkEndpoints, ProviderError};
use strata_identifiers::EpochCommitment;

use super::OLChainStore;
use crate::chain_spec::OLChainSpec;
use crate::graph_types::{OLCheckpointLink, OLStateNode};

/// Fetches a checkpoint link, which exists once both the epoch's summary and
/// its L1-observed payload are known.
pub(super) fn fetch_link(
    store: &impl OLChainStore,
    epoch: &EpochCommitment,
) -> Result<Option<OLCheckpointLink>, ProviderError> {
    let Some(summary) = store.fetch_epoch_summary(epoch)? else {
        return Ok(None);
    };
    let Some(payload) = store.fetch_checkpoint_payload(epoch)? else {
        return Ok(None);
    };
    Ok(Some(OLCheckpointLink::new(summary, payload)))
}

/// The nodes a checkpoint link connects: the previous epoch's terminal and
/// this one's.  The genesis epoch's checkpoint departs from nowhere, so it
/// isn't a link.
pub(super) fn fetch_endpoints(
    store: &impl OLChainStore,
    epoch: &EpochCommitment,
) -> Result<Option<LinkEndpoints<OLChainSpec>>, ProviderError> {
    let Some(summary) = store.fetch_epoch_summary(epoch)? else {
        return Ok(None);
    };
    let Some(prev) = summary.get_prev_epoch_commitment() else {
        return Ok(None);
    };
    let Some(prev_summary) = store.fetch_epoch_summary(&prev)? else {
        return Ok(None);
    };
    Ok(Some(LinkEndpoints::new(
        OLStateNode::from_epoch_summary(&prev_summary),
        OLStateNode::from_epoch_summary(&summary),
    )))
}
