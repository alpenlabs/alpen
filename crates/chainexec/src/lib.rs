//! Low level OL block executor.
//!
//! This handle all of the logic for executing the Strata orchestration layer
//! chain, both classically and via checkpoint DA updates.
//!
//! It is intended to be portable so that it can used both by full nodes,
//! checkpoint sync nodes, and proofs.  It does minimal state tracking of its
//! own and is expected to be driven primarily from the outside.

mod diff;
mod errors;
mod executor;
mod output;
mod state_access;
mod tip_state;
mod traits;
mod validation_util;

pub use diff::ChangedState;
pub use errors::{Error, ExecResult};
pub use executor::ChainExecutor;
pub use output::{BlockExecutionOutput, EpochExecutionOutput, LogMessage};
pub use state_access::MemStateAccessor;
pub use tip_state::TipState;
pub use traits::ExecContext;
