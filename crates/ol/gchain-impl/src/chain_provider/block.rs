//! Block links in the chain graph.

use strata_gchain_types::{LinkEndpoints, ProviderError};
use strata_identifiers::OLBlockCommitment;
use strata_ol_chain_types_v1::OLBlockHeaderV1;

use super::OLChainStore;
use crate::chain_spec::OLChainSpec;
use crate::graph_types::{OLBlockLink, OLStateNode};

/// Fetches a block's header, checking it's at the slot the commitment says.
pub(super) fn fetch_header(
    store: &impl OLChainStore,
    block: &OLBlockCommitment,
) -> Result<Option<OLBlockHeaderV1>, ProviderError> {
    let Some(header) = store.fetch_header(block.blkid())? else {
        return Ok(None);
    };
    if header.slot() != block.slot() {
        return Err(ProviderError::Malformed(format!(
            "header for {block} stored at slot {}",
            header.slot()
        )));
    }
    Ok(Some(header))
}

/// Fetches a block link, which needs the parent header to exist too.
pub(super) fn fetch_link(
    store: &impl OLChainStore,
    block: &OLBlockCommitment,
) -> Result<Option<OLBlockLink>, ProviderError> {
    let Some(block) = store.fetch_block(block.blkid())? else {
        return Ok(None);
    };
    let parent_header = match fetch_parent_header(store, block.header())? {
        ParentHeader::Genesis => None,
        ParentHeader::Known(ph) => Some(ph),
        ParentHeader::Unknown => return Ok(None),
    };
    Ok(Some(OLBlockLink::new(block, parent_header)))
}

/// The nodes a block link connects, which its own header and its parent's
/// describe.  The genesis block departs from nowhere, so it isn't a link.
pub(super) fn fetch_endpoints(
    store: &impl OLChainStore,
    block: &OLBlockCommitment,
) -> Result<Option<LinkEndpoints<OLChainSpec>>, ProviderError> {
    let Some(header) = fetch_header(store, block)? else {
        return Ok(None);
    };
    let origin = match fetch_parent_header(store, &header)? {
        ParentHeader::Known(ph) => OLStateNode::from_header(&ph),
        ParentHeader::Genesis | ParentHeader::Unknown => return Ok(None),
    };
    Ok(Some(LinkEndpoints::new(
        origin,
        OLStateNode::from_header(&header),
    )))
}

enum ParentHeader {
    Genesis,
    Known(OLBlockHeaderV1),
    Unknown,
}

fn fetch_parent_header(
    store: &impl OLChainStore,
    header: &OLBlockHeaderV1,
) -> Result<ParentHeader, ProviderError> {
    if header.is_genesis_slot() {
        return Ok(ParentHeader::Genesis);
    }
    Ok(match store.fetch_header(header.parent_blkid())? {
        Some(ph) => ParentHeader::Known(ph),
        None => ParentHeader::Unknown,
    })
}
