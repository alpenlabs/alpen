//! Generic block accumulation and sealing policy framework.
//!
//! An [`AccumulationPolicy`] defines:
//! - The type of data collected per block ([`AccumulationPolicy::BlockData`])
//! - The type of accumulated value ([`AccumulationPolicy::AccumulatedValue`])
//! - How to accumulate block data ([`AccumulationPolicy::accumulate`])
//!
//! A [`SealingPolicy`] determines when to seal based on the accumulated state.
//!
//! These traits are used by the batch builder (and will be used by the chunk
//! builder) with different policy implementations.
//!
//! # Built-in policies
//!
//! | Module | Seals when… |
//! |--------|-------------|
//! | [`block_count_policy`] | Block count reaches a threshold |
//! | [`gas_limit_policy`] | Cumulative gas exceeds a limit |
//! | [`or_policy`] | Either of two composed policies triggers |

pub mod block_count_policy;
pub mod gas_limit_policy;
pub mod or_policy;
mod policy;

pub use policy::{AccumulationPolicy, Accumulator, BlockDataProvider, SealingPolicy};
