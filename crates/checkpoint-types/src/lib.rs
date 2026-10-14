//! Checkpoint-related types for the Strata rollup.

mod batch;
mod prover_task;
mod spec_after_epoch;
mod terminal_header;

pub use batch::*;
pub use prover_task::CheckpointProofTask;
pub use spec_after_epoch::{
    select_spec_after_epoch, select_spec_after_epoch_async, SelectSpecAfterEpochError,
};
pub use terminal_header::*;
