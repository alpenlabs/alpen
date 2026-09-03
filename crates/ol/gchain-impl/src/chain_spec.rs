use strata_gchain_types::GChainSpec;
use strata_identifiers::OLBlockCommitment;

use crate::graph_types::*;

#[derive(Copy, Clone, Debug, Hash)]
pub struct OLChainSpec;

impl GChainSpec for OLChainSpec {
    // actually fine that these are the same here, for now
    type NodeRef = OLStateNode;
    type Node = OLStateNode;

    type LinkRef = OLLinkRef;
    type LinkHeader = OLLinkHeader;
    type Link = OLLink;

    fn get_header_ref(nh: &Self::LinkHeader) -> Self::LinkRef {
        match nh {
            OLLinkHeader::BlockV1(header) => header.compute_block_commitment().into(),
            OLLinkHeader::Checkpoint(ckpt) => ckpt.get_epoch_commitment().into(),
        }
    }

    fn get_header_canonical_prev(nh: &Self::LinkHeader) -> Option<Self::LinkRef> {
        match nh {
            // The genesis block is the start of the graph, so it has no
            // predecessor link.
            OLLinkHeader::BlockV1(header) if header.is_genesis_slot() => None,

            // Slots are strictly sequential in OL v1, so the parent block is
            // always at the immediately preceding slot.
            OLLinkHeader::BlockV1(header) => {
                let parent = OLBlockCommitment::new(header.slot() - 1, *header.parent_blkid());
                Some(parent.into())
            }

            // The checkpoint for the previous epoch, unless this is the genesis
            // epoch's checkpoint.
            OLLinkHeader::Checkpoint(ckpt) => ckpt.get_prev_epoch_commitment().map(Into::into),
        }
    }
}
