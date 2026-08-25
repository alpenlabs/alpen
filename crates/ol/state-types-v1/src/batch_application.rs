//! Trait for states that can have write batches applied.

use strata_ol_state_types::{IStateAccessor, StateResult};

use crate::{OLAccountStateV1, WriteBatch};

/// Trait for states that can have write batches applied.
///
/// This trait requires `IStateAccessor` and adds atomic batch application
/// capability. Not all state accessors support this - for example,
/// `WriteTrackingState` only tracks writes and cannot apply them directly
/// since it holds an immutable reference to the base state.
pub trait IStateBatchApplicable: IStateAccessor {
    /// Applies a write batch to this state atomically.
    ///
    /// This updates the global state, epochal state, and ledger accounts
    /// with the modifications from the batch.
    ///
    /// If this returns an error then the state is left unmodified.
    fn apply_write_batch(&mut self, batch: WriteBatch<OLAccountStateV1>) -> StateResult<()>;
}
