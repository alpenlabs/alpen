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

use std::{boxed::Box, vec::Vec};

use async_trait::async_trait;
use strata_primitives::l1::L1BlockCommitment;

/// Interface to submit blocks to CSM in blocking or async fashion.
// TODO reverse the convention on these function names, since you can't
// accidentally call an async fn in a blocking context
#[async_trait]
pub trait BlockSubmitter: Send + Sync {
    /// Submit block blocking
    fn submit_block(&self, block: L1BlockCommitment) -> anyhow::Result<()>;
    /// Submit block async
    async fn submit_block_async(&self, block: L1BlockCommitment) -> anyhow::Result<()>;
}

/// A glue implementation to allow several block submitters "consume" from the same reader.
#[allow(missing_debug_implementations)]
pub struct CombinedBlockSubmitter {
    submitters: Vec<std::sync::Arc<dyn BlockSubmitter>>,
}

#[async_trait]
impl BlockSubmitter for CombinedBlockSubmitter {
    /// Sends a new l1 block to the csm machinery.
    fn submit_block(&self, block: L1BlockCommitment) -> anyhow::Result<()> {
        for s in self.submitters.iter() {
            s.submit_block(block)?;
        }

        Ok(())
    }

    /// Sends a new l1 block to the csm machinery.
    async fn submit_block_async(&self, block: L1BlockCommitment) -> anyhow::Result<()> {
        for s in self.submitters.iter() {
            s.submit_block_async(block).await?;
        }

        Ok(())
    }
}

impl CombinedBlockSubmitter {
    pub fn new(submitters: Vec<std::sync::Arc<dyn BlockSubmitter>>) -> Self {
        Self { submitters }
    }
}

pub mod prelude;

pub use strata_primitives::batch;
