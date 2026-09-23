//! The OL chain graph as seen through a chain store.

mod block;
mod checkpoint;

use strata_asm_checkpoint_types::CheckpointPayload;
use strata_checkpoint_types::EpochSummary;
use strata_gchain_types::{ChainProvider, GLink, LinkEndpoints, ProviderError};
use strata_identifiers::{Epoch, EpochCommitment, OLBlockCommitment, OLBlockId, Slot};
use strata_ol_chain_types_v1::{OLBlockHeaderV1, OLBlockV1};

use crate::chain_spec::OLChainSpec;
use crate::graph_types::{CheckpointData, OLLink, OLLinkHeader, OLLinkRef, OLStateNode};

/// What the chain provider needs from underlying OL storage.
///
/// Reads only; the graph is built up by whatever ingests blocks and
/// checkpoints, and the provider just exposes it.
pub trait OLChainStore: Send + Sync + 'static {
    fn fetch_block(&self, blkid: &OLBlockId) -> Result<Option<OLBlockV1>, ProviderError>;

    /// Fetches a block's header, including terminal headers reconstructed from
    /// checkpoints, which have no block.
    fn fetch_header(&self, blkid: &OLBlockId) -> Result<Option<OLBlockHeaderV1>, ProviderError>;

    fn fetch_blocks_at_slot(&self, slot: Slot) -> Result<Vec<OLBlockId>, ProviderError>;

    fn fetch_epoch_summary(
        &self,
        epoch: &EpochCommitment,
    ) -> Result<Option<EpochSummary>, ProviderError>;

    /// Fetches every known summary for an epoch number; there can be more than
    /// one across forks.
    fn fetch_epoch_summaries_at(&self, epoch: Epoch) -> Result<Vec<EpochSummary>, ProviderError>;

    /// Fetches the L1-observed checkpoint payload for an epoch, if one has
    /// been seen.
    fn fetch_checkpoint_payload(
        &self,
        epoch: &EpochCommitment,
    ) -> Result<Option<CheckpointPayload>, ProviderError>;
}

/// [`ChainProvider`] for the OL chain graph over an [`OLChainStore`].
pub struct OLChainProvider<C: OLChainStore> {
    store: C,
}

impl<C: OLChainStore> OLChainProvider<C> {
    pub fn new(store: C) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &C {
        &self.store
    }
}

impl<C: OLChainStore> ChainProvider for OLChainProvider<C> {
    type Spec = OLChainSpec;

    fn fetch_link_header(&self, lref: &OLLinkRef) -> Result<Option<OLLinkHeader>, ProviderError> {
        match lref {
            OLLinkRef::Block(info) => {
                Ok(block::fetch_header(&self.store, info.block())?.map(OLLinkHeader::Block))
            }
            OLLinkRef::Checkpoint(info) => Ok(self
                .store
                .fetch_epoch_summary(info.epoch())?
                .map(|s| OLLinkHeader::Checkpoint(CheckpointData::new(s)))),
        }
    }

    fn fetch_link(&self, lref: &OLLinkRef) -> Result<Option<OLLink>, ProviderError> {
        let link = match lref {
            OLLinkRef::Block(info) => {
                block::fetch_link(&self.store, info.block())?.map(OLLink::Block)
            }
            OLLinkRef::Checkpoint(info) => {
                checkpoint::fetch_link(&self.store, info.epoch())?.map(OLLink::Checkpoint)
            }
        };

        match link {
            Some(link) if !link.check_structurally_consistent() => {
                Err(ProviderError::InconsistentLink(format!("{lref:?}")))
            }
            link => Ok(link),
        }
    }

    fn fetch_link_endpoints(
        &self,
        lref: &OLLinkRef,
    ) -> Result<Option<LinkEndpoints<OLChainSpec>>, ProviderError> {
        match lref {
            OLLinkRef::Block(info) => block::fetch_endpoints(&self.store, info.block()),
            OLLinkRef::Checkpoint(info) => checkpoint::fetch_endpoints(&self.store, info.epoch()),
        }
    }

    fn fetch_forward_links(&self, nref: &OLStateNode) -> Result<Vec<OLLinkRef>, ProviderError> {
        let mut links = Vec::new();

        // Blocks at the next slot that build on this one.
        let next_slot = nref.block().slot() + 1;
        for blkid in self.store.fetch_blocks_at_slot(next_slot)? {
            let Some(header) = self.store.fetch_header(&blkid)? else {
                continue;
            };
            if header.parent_blkid() == nref.block().blkid() {
                links.push(OLBlockCommitment::new(next_slot, blkid).into());
            }
        }

        // The next epoch's checkpoint, if this is its previous terminal.
        for summary in self.store.fetch_epoch_summaries_at(nref.epoch() + 1)? {
            if summary.prev_terminal() == nref.block() {
                links.push(summary.get_epoch_commitment().into());
            }
        }

        Ok(links)
    }

    fn fetch_backward_links(&self, nref: &OLStateNode) -> Result<Vec<OLLinkRef>, ProviderError> {
        let mut links = Vec::new();

        // The block that produced this state, unless it's genesis, which has
        // no origin to link from.
        if let Some(header) = block::fetch_header(&self.store, nref.block())?
            && !header.is_genesis_slot()
        {
            links.push((*nref.block()).into());
        }

        // The checkpoint for the epoch this terminates, if there is one.
        let epoch = EpochCommitment::from_terminal(nref.epoch(), *nref.block());
        if let Some(summary) = self.store.fetch_epoch_summary(&epoch)?
            && summary.get_prev_epoch_commitment().is_some()
        {
            links.push(summary.get_epoch_commitment().into());
        }

        Ok(links)
    }
}
