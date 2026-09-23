//! Interfaces the OL processor stages use to reach their aggregated state.
//!
//! A stage's aggregated state is whatever it committed; these are the narrow
//! capabilities each stage needs to read and update it.  Concrete storage
//! lives with the node, not here.

use strata_asm_common::AsmManifest;
use strata_gchain_types::ProcError;
use strata_identifiers::L1Height;
use strata_ol_chain_types_v1::OLBlockHeaderV1;
use strata_ol_state_support_types::{IComputeStateRootWithWrites, IndexerWrites};
use strata_ol_state_types::IStateAccessor;
use strata_ol_state_types_v1::{OLAccountStateV1, WriteBatch};

use crate::graph_types::{OLLinkRef, OLStateNode};

/// Store for the committed OL states the exec stage maintains.
///
/// A state is kept at every node the stage has committed a path to, until the
/// stage prunes it, so that any path since the prune point can still be rolled
/// back to the state it departed from.
///
/// The stages only ever read a committed state through the accessor traits
/// and describe a new one as a diff from an old one.  The store decides how
/// states are actually laid out, which is going to change once the accounts
/// table is split out and accessed lazily.
pub trait OLStateStore: Send + Sync + 'static {
    /// A committed state, read-only.
    type State: IStateAccessor<AccountState = OLAccountStateV1> + IComputeStateRootWithWrites;

    /// Fetches the committed state at a node, if the stage committed one there.
    fn fetch_state(&self, node: &OLStateNode) -> Result<Option<Self::State>, ProcError>;

    /// Stores the state at `target` as the state at `base` with `diffs`
    /// applied in order.  Idempotent.
    fn derive_state(
        &self,
        base: &OLStateNode,
        target: &OLStateNode,
        diffs: &[&WriteBatch],
    ) -> Result<(), ProcError>;

    /// Discards the committed state at a node, such as when the path reaching
    /// it is rolled back.
    fn delete_state(&self, node: &OLStateNode) -> Result<(), ProcError>;

    /// Discards every committed state older than the node, which is becoming
    /// the oldest one anything can roll back to.
    fn prune_states_before(&self, node: &OLStateNode) -> Result<(), ProcError>;

    /// Stores a terminal header reconstructed from a checkpoint.
    ///
    /// These are only produced for checkpoint steps, since there's no block to
    /// take the header from, and later block steps need them as parent headers.
    // TODO(trey): rework this so that we assume it already exists from the persisted checkpoint data from the ASM runs
    fn store_terminal_header(&self, header: &OLBlockHeaderV1) -> Result<(), ProcError>;
}

/// Source of the ASM manifests a checkpoint's epoch covers.
///
/// Block steps carry their manifests in the block body, but a checkpoint step
/// only names the L1 range, so the stage fetches them separately.
pub trait L1ManifestProvider: Send + Sync + 'static {
    /// Fetches the manifest for an L1 height, if it's known yet.
    fn fetch_manifest(&self, height: L1Height) -> Result<Option<AsmManifest>, ProcError>;
}

/// Store for the state index the index stage maintains.
pub trait OLIndexStore: Send + Sync + 'static {
    /// Applies the index writes a link produced.  Idempotent for the same
    /// link.
    fn apply_index_writes(&self, lref: &OLLinkRef, writes: &IndexerWrites)
    -> Result<(), ProcError>;

    /// Reverts the index writes a link produced.
    fn revert_index_writes(&self, lref: &OLLinkRef) -> Result<(), ProcError>;
}
