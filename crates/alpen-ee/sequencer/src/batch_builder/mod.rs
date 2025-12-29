//! Batch builder for creating batches from executed blocks.
//!
//! The batch builder monitors the canonical chain and accumulates blocks into batches
//! based on a configurable sealing policy. It handles chain reorgs by detecting when
//! previously sealed batches are no longer on the canonical chain and reverting them.
//!
//! # Architecture
//!
//! The batch builder is parameterized by a [`BatchPolicy`] that defines:
//! - The type of data collected per block ([`BatchPolicy::BlockData`])
//! - The type of accumulated value ([`BatchPolicy::AccumulatedValue`])
//! - How to accumulate block data ([`BatchPolicy::accumulate`])
//!
//! A [`BatchSealingPolicy`] determines when to seal a batch based on the accumulated state.
//!
//! # Reorg Handling
//!
//! Two types of reorgs are handled:
//! - **Deep reorg**: The last sealed batch's end block is no longer canonical. Batches are reverted
//!   in storage and state is reset.
//! - **Shallow reorg**: Only blocks in the accumulator (not yet in a batch) are affected. The
//!   accumulator is simply reset.
//!
//! # Usage
//!
//! Use [`BatchBuilderBuilder`] to construct the task:
//!
//! ```ignore
//! use alpen_ee_sequencer::{
//!     BatchBuilderBuilder, BatchBuilderState, BlockCountPolicy, FixedBlockCountSealing,
//!     init_batch_builder_state,
//! };
//!
//! let state: BatchBuilderState<BlockCountPolicy> =
//!     init_batch_builder_state(genesis_hash, &batch_storage).await?;
//!
//! let sealing = FixedBlockCountSealing::new(100);
//!
//! let task = BatchBuilderBuilder::new(
//!     genesis_hash,
//!     state,
//!     preconf_rx,
//!     block_data_provider,
//!     sealing,
//!     block_storage,
//!     batch_storage,
//!     exec_chain,
//! )
//! .with_max_blocks_per_batch(100)
//! .build();
//!
//! task.await;
//! ```

mod accumulator;
mod block_count;
mod config;
mod ctx;
mod handle;
mod state;
mod task;
mod traits;

pub use accumulator::Accumulator;
pub use block_count::{BlockCountData, BlockCountPolicy, BlockCountValue, FixedBlockCountSealing};
pub use config::BatchBuilderConfig;
pub use handle::{BatchBuilderBuilder, BatchBuilderHandle};
pub use state::{init_batch_builder_state, BatchBuilderState};
pub use traits::{BatchPolicy, BatchSealingPolicy, BlockDataProvider};
