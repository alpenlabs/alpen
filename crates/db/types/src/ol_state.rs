//! Toplevel OL state database interface.

// TODO(STR-4220): replace OLStateV1 with a versionable wrapper
// TODO(STR-4220): make WriteBatch use its own concrete account type instead of being generic

#[cfg(feature = "proxies")]
use strata_db_macros::gen_proxy;
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types_v1::{OLAccountStateV1, OLStateV1, WriteBatch};

#[cfg(feature = "proxies")]
use crate::DbError;
use crate::DbResult;

/// Database trait for toplevel OL state storage.
///
/// Stores OLStateV1 snapshots keyed by OLBlockCommitment (block ID + slot).
/// This allows retrieving state for any block in the chain.
#[cfg_attr(
    feature = "proxies",
    gen_proxy(error = DbError, tracing_component = "storage:ol_state")
)]
pub trait OLStateDatabase: Send + Sync + 'static {
    /// Stores a toplevel OLStateV1 snapshot for a given block commitment.
    fn put_toplevel_ol_state(
        &self,
        commitment: OLBlockCommitment,
        state: OLStateV1,
    ) -> DbResult<()>;

    /// Retrieves a toplevel OLStateV1 snapshot for a given block commitment.
    fn get_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<Option<OLStateV1>>;

    /// Gets the latest toplevel OLStateV1 (highest slot).
    fn get_latest_toplevel_ol_state(&self) -> DbResult<Option<(OLBlockCommitment, OLStateV1)>>;

    /// Deletes a toplevel OLStateV1 snapshot for a given block commitment.
    fn del_toplevel_ol_state(&self, commitment: OLBlockCommitment) -> DbResult<()>;

    /// Stores an OL write batch for a given block commitment.
    ///
    /// Write batches represent state changes that can be applied to a state.
    fn put_ol_write_batch(
        &self,
        commitment: OLBlockCommitment,
        wb: WriteBatch<OLAccountStateV1>,
    ) -> DbResult<()>;

    /// Retrieves an OL write batch for a given block commitment.
    fn get_ol_write_batch(
        &self,
        commitment: OLBlockCommitment,
    ) -> DbResult<Option<WriteBatch<OLAccountStateV1>>>;

    /// Deletes an OL write batch for a given block commitment.
    fn del_ol_write_batch(&self, commitment: OLBlockCommitment) -> DbResult<()>;
}
