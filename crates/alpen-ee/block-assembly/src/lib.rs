//! Handles assembly of EE blocks.

mod block;
mod package;
mod payload;
mod record;

pub use block::{build_next_exec_block, BlockAssemblyInputs, BlockAssemblyOutputs};
pub use record::{assemble_next_exec_block_record, AssembleExecBlockInputs, AssembledExecBlock};
