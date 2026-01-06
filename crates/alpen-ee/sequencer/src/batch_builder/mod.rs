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
//! Four reorg scenarios are handled:
//! - **No reorg**: The tip is canonical, no action needed.
//! - **Shallow reorg**: Only blocks in the accumulator or pending queue are affected. The
//!   accumulator and pending queue are reset, but no batches are reverted.
//! - **Batch reorg**: The last sealed batch's end block is no longer canonical. Batches are
//!   reverted in storage to the last canonical batch and state is reset.
//! - **Deep reorg**: The reorg extends below finalized batches. This requires manual intervention
//!   as finalized batches cannot be automatically reverted.
//!
//! # Usage
//!
//! Use [`create_batch_builder`] to construct the task.
//!
//! **Important**: [`alpen_ee_genesis::ensure_batch_genesis`] must be called before
//! [`init_batch_builder_state`] to ensure the genesis batch exists in storage.
//!
//! ```ignore
//! use alpen_ee_genesis::ensure_batch_genesis;
//! use alpen_ee_sequencer::{
//!     create_batch_builder, BatchBuilderState, BlockCountPolicy, FixedBlockCountSealing,
//!     init_batch_builder_state,
//! };
//!
//! // Ensure genesis batch exists (must be called before init_batch_builder_state)
//! ensure_batch_genesis(&config, &batch_storage).await?;
//!
//! // Initialize state from storage
//! let state: BatchBuilderState<BlockCountPolicy> =
//!     init_batch_builder_state(&batch_storage).await?;
//!
//! let sealing = FixedBlockCountSealing::new(100);
//!
//! let (handle, task) = create_batch_builder(
//!     initial_batch_id,
//!     genesis,
//!     state,
//!     preconf_rx,
//!     block_data_provider,
//!     sealing,
//!     block_storage,
//!     batch_storage,
//!     exec_chain,
//! );
//!
//! // Use handle to watch for batch updates
//! let watcher = handle.latest_batch_watcher();
//!
//! // Run the task
//! task.await;
//! ```

mod accumulator;
mod block_count;
mod canonical;
mod ctx;
mod handle;
mod reorg;
mod state;
mod task;
mod traits;

pub use accumulator::Accumulator;
pub use block_count::{BlockCountData, BlockCountPolicy, BlockCountValue, FixedBlockCountSealing};
pub use handle::{create_batch_builder, BatchBuilderHandle};
pub use state::{init_batch_builder_state, BatchBuilderState};
pub use traits::{BatchPolicy, BatchSealingPolicy, BlockDataProvider};
