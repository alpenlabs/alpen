//! Update submitter task for submitting batches to the OL chain.
//!
//! This module monitors batches in `ProofReady` state and submits them as
//! `SnarkAccountUpdate` transactions to the OL chain.

mod task;
mod update_builder;

pub use task::create_update_submitter_task;
