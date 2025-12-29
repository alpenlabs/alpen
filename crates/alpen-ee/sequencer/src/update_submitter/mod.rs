//! Update submitter task for submitting batches to the OL chain.
//!
//! This module monitors batches in `ProofReady` state and submits them as
//! `SnarkAccountUpdate` transactions to the OL chain.

mod task;

pub use task::update_submitter_task;
