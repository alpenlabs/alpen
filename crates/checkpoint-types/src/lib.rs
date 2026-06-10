//! Checkpoint-related types for the Strata rollup.

mod batch;
mod prover_task;
mod terminal_header;

pub use batch::*;
pub use prover_task::CheckpointProofTask;
pub use terminal_header::*;
