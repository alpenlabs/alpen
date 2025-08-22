#![allow(stable_features)] // FIX: this is needed for sp1 toolchain.
#![feature(is_sorted, is_none_or)]

//! Rollup types relating to the consensus-layer state of the rollup.
//!
//! Types relating to the execution-layer state are kept generic, not
//! reusing any Reth types.

pub mod block;
pub mod block_validation;
pub mod bridge_ops;
pub mod bridge_state;
pub mod chain_state;
pub mod client_state;
pub mod exec_env;
pub mod exec_update;
pub mod forced_inclusion;
pub mod genesis;
pub mod header;
pub mod id;
pub mod l1;
pub mod operation;
pub mod state_op;
pub mod state_queue;

use async_trait::async_trait;
use strata_primitives::l1::L1BlockCommitment;

/// Interface to submit event to CSM in blocking or async fashion.
// TODO reverse the convention on these function names, since you can't
// accidentally call an async fn in a blocking context
#[async_trait]
pub trait BlockSubmitter {
    /// Submit event blocking
    fn submit_event(&self, sync_event: L1BlockCommitment) -> anyhow::Result<()>;
    /// Submit event async
    async fn submit_event_async(&self, sync_event: L1BlockCommitment) -> anyhow::Result<()>;
}

pub mod prelude;

pub use strata_primitives::batch;
