//! Toplevel OL state database interface.

// TODO(trey): replace OLState with a versionable wrapper
// TODO(trey): make WriteBatch use its own concrete account type instead of being generic

#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Database trait for toplevel OL state storage.
///
/// Stores OLState snapshots keyed by OLBlockCommitment (block ID + slot).
/// This allows retrieving state for any block in the chain.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:ol_state")
)]
pub trait OLStateDatabase: Send + Sync + 'static {
    /// Stores a toplevel OLState snapshot for a given block commitment.
    fn put_toplevel_ol_state(&self, commitment: OLBlockCommitment, state: OLState) -> DbResult<()>;

    /// Retrieves a toplevel OLState snapshot for a given block commitment.
    fn get_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<Option<OLState>>;

    /// Gets the latest toplevel OLState (highest slot).
    fn get_latest_toplevel_ol_state(&self) -> DbResult<Option<(OLBlockCommitment, OLState)>>;

    /// Deletes a toplevel OLState snapshot for a given block commitment.
    fn del_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<()>;

    /// Stores an OL write batch for a given block commitment.
    ///
    /// Write batches represent state changes that can be applied to a state.
    fn put_ol_write_batch(
        &self,
        commitment: OLBlockCommitment,
        wb: WriteBatch<OLAccountState>,
    ) -> DbResult<()>;

    /// Retrieves an OL write batch for a given block commitment.
    fn get_ol_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<WriteBatch<OLAccountState>>>;

    /// Deletes an OL write batch for a given block commitment.
    fn del_ol_write_batch(&self, commitment: OLBlockCommitment) -> DbResult<()>;
}
