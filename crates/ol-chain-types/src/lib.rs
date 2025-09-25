//! Orchestration Layer (OL) chain-specific types for the Strata rollup.
//!
//! This crate contains OL chain-specific types that are independent of
//! the state management layer.

mod block;
mod header;
mod id;
mod validation;

pub use block::*;
pub use header::*;
pub use id::*;
pub use validation::*;
