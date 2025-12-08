//! Handles assembly of EE blocks.

mod block;
mod package;
mod payload;

pub use block::{build_next_exec_block, BlockAssemblyInputs, BlockAssemblyOutputs};
