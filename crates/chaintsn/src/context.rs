//! Interfaces to expose the context in which a block is being validated.

use strata_primitives::l1::{L1BlockCommitment, L1BlockManifest};
use strata_state::{header::L2BlockHeader, id::L2BlockId};

use crate::errors::{ProviderError, ProviderResult};

/// Provider for context about the block in the chain.
///
/// Does NOT provide access to chainstate information.  This is primarily
/// involving block headers.  It will probably also provide L1 manifests.
pub trait BlockContext {
    /// Returns the slot that we're checking.
    fn slot(&self) -> u64;

    /// Returns the unix millis timestamp of the block.
    fn timestamp(&self) -> u64;

    /// Returns the parent block's ID.
    fn parent_blkid(&self) -> &L2BlockId;

    /// Returns the parent block's header.
    fn parent_header(&self) -> &L2BlockHeader;
}

/// Provider for queries to the backing state we're building on top of.
pub trait StateProvider {
    // TODO
}

/// Provider for queries to sideloaded state like L1 block manifests.
pub trait AuxProvider {
    /// Fetches an L1 block manifest.
    fn get_l1_block_manifest(&self, block: &L1BlockCommitment) -> ProviderResult<L1BlockManifest>;
}
