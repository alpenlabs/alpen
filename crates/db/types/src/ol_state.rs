//! Toplevel OL state database interface.

#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_container::OLStateContainer;
use strata_ol_state_types_v1::WriteBatch;

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Database trait for toplevel OL state storage.
///
/// Stores [`OLStateContainer`] snapshots keyed by [`OLBlockCommitment`] (block
/// ID + slot), so a snapshot keeps its spec versions and reproduces the
/// committed state root. This allows retrieving state for any block in the
/// chain.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:ol_state")
)]
pub trait OLStateDatabase: Send + Sync + 'static {
    /// Stores a toplevel OL state snapshot for a given block commitment.
    fn put_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
        state: OLStateContainer,
    ) -> DbResult<()>;

    /// Retrieves a toplevel OL state snapshot for a given block commitment.
    fn get_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<OLStateContainer>>;

    /// Gets the latest toplevel OL state snapshot (highest slot).
    fn get_latest_toplevel_ol_state(
        &self,
    ) -> DbResult<Option<(OLBlockCommitment, OLStateContainer)>>;

    /// Deletes a toplevel OL state snapshot for a given block commitment.
    fn del_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<()>;

    /// Stores an OL write batch for a given block commitment.
    ///
    /// Write batches represent state changes that can be applied to a state.
    fn put_ol_write_batch(&self, commitment: OLBlockCommitment, wb: WriteBatch) -> DbResult<()>;

    /// Retrieves an OL write batch for a given block commitment.
    fn get_ol_write_batch(&self, commitment: OLBlockCommitment) -> DbResult<Option<WriteBatch>>;

    /// Deletes an OL write batch for a given block commitment.
    fn del_ol_write_batch(&self, commitment: OLBlockCommitment) -> DbResult<()>;
}
