//! Types relating to chainstate database.
//!
//! This is in a separate module since it's a complex database due to the large
//! size of structures.
//!
//! The assumption we have here is that full chainstate instances may be large
//! structures, so we want to avoid materializing them in full.  Instead, we
//! maintain a relatively small number of full state snapshots and a set of
//! changesets to them.  This is similar to a write-ahead log, but it's a layer
//! up and we reason about merging logs entries explicitly.
//!
//! The structure of the state now is just the "toplevel" [``Chainstate``]
//! struct.  In the future, we will have a "bulk" state that contains
//! potentially-many ledger entries.

use strata_ol_chainstate_types::{Chainstate, WriteBatch};
use strata_identifiers::Buf32;

use crate::DbResult;

/// ID of a state instance.
pub type StateInstanceId = u64;

/// ID we use to query write batches.
///
/// This is likely to just be a block ID, but we might make it more general.
pub type WriteBatchId = Buf32;

/// New chainstate database trait.
pub trait ChainstateDatabase: Send + Sync + 'static {
    /// Creates a new empty state instance with a certain toplevel state and
    /// empty bulk state.
    fn create_new_inst(&self, toplevel: Chainstate) -> DbResult<StateInstanceId>;

    /// Clones a state instance to create a new state instance.
    ///
    /// This MAY be a copy-on-write type clone.
    fn clone_inst(&self, id: StateInstanceId) -> DbResult<StateInstanceId>;

    /// Deletes a state instance, allowing its underlying data to be freed.
    fn del_inst(&self, id: StateInstanceId) -> DbResult<()>;

    /// Gets a list of state instance IDs.
    fn get_insts(&self) -> DbResult<Vec<StateInstanceId>>;

    /// Gets the root of a state instance.
    fn get_inst_root(&self, id: StateInstanceId) -> DbResult<Buf32>;

    /// Gets the toplevel state for a snapshot.
    fn get_inst_toplevel_state(&self, id: StateInstanceId) -> DbResult<Chainstate>;

    /// Puts a write batch associated with an opaque ID.
    ///
    /// This is likely to be a block ID, but can be something else.
    fn put_write_batch(&self, id: WriteBatchId, wb: WriteBatch) -> DbResult<()>;

    /// Gets a write batch associated with some ID, if it exists.
    fn get_write_batch(&self, id: WriteBatchId) -> DbResult<Option<WriteBatch>>;

    /// Deletes a write batch.
    fn del_write_batch(&self, id: WriteBatchId) -> DbResult<()>;

    /// Applies a sequence of write batches to a state in a single atomic
    /// operation.  If there is a failure in applying the write ops, the state
    /// instance MUST remain unchanged.
    fn merge_write_batches(
        &self,
        state_id: StateInstanceId,
        wb_ids: Vec<WriteBatchId>,
    ) -> DbResult<()>;

    // TODO add accessor functions for fetching bulk state values
}
