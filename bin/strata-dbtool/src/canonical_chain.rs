//! Canonical chain view selected by a command's `--node-role`.

use strata_db_types::DbResult;
use strata_identifiers::{Epoch, EpochCommitment, OLBlockCommitment, Slot};
use strata_ol_canonical_chain_provider::{
    FinalizedOLChainProvider, OLCanonicalChainProvider, OLCanonicalChainProviderImpl,
};
use strata_storage::NodeStorage;

use crate::cli::NodeRole;

/// Canonical chain provider for the node whose database is being inspected.
pub(crate) enum CanonicalChainProvider {
    /// The canonical block index written by fork choice.
    FcmIndex(OLCanonicalChainProviderImpl),

    /// Finality from client state and L1 checkpoint observations.
    L1Finalized(FinalizedOLChainProvider),
}

impl CanonicalChainProvider {
    /// Selects the view a node with `node_role` uses.
    pub(crate) fn for_node_role(storage: &NodeStorage, node_role: NodeRole) -> Self {
        match node_role {
            NodeRole::Sequencer => Self::FcmIndex(OLCanonicalChainProviderImpl::new(storage)),
            NodeRole::CheckpointSync => Self::L1Finalized(FinalizedOLChainProvider::new(storage)),
        }
    }
}

/// Runs `$call` on the provider `$self` holds.
macro_rules! delegate_to_provider {
    ($self:ident, $provider:ident => $call:expr) => {
        match $self {
            Self::FcmIndex($provider) => $call,
            Self::L1Finalized($provider) => $call,
        }
    };
}

impl OLCanonicalChainProvider for CanonicalChainProvider {
    async fn get_canonical_block_at_async(
        &self,
        slot: Slot,
    ) -> DbResult<Option<OLBlockCommitment>> {
        delegate_to_provider!(self, provider => provider.get_canonical_block_at_async(slot).await)
    }

    fn get_canonical_block_at_blocking(&self, slot: Slot) -> DbResult<Option<OLBlockCommitment>> {
        delegate_to_provider!(self, provider => provider.get_canonical_block_at_blocking(slot))
    }

    async fn get_canonical_tip_async(&self) -> DbResult<Option<OLBlockCommitment>> {
        delegate_to_provider!(self, provider => provider.get_canonical_tip_async().await)
    }

    fn get_canonical_tip_blocking(&self) -> DbResult<Option<OLBlockCommitment>> {
        delegate_to_provider!(self, provider => provider.get_canonical_tip_blocking())
    }

    async fn get_canonical_epoch_commitment_at_async(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        delegate_to_provider!(
            self,
            provider => provider.get_canonical_epoch_commitment_at_async(epoch).await
        )
    }

    fn get_canonical_epoch_commitment_at_blocking(
        &self,
        epoch: Epoch,
    ) -> DbResult<Option<EpochCommitment>> {
        delegate_to_provider!(
            self,
            provider => provider.get_canonical_epoch_commitment_at_blocking(epoch)
        )
    }
}
