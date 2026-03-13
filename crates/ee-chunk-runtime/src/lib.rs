//! Chunk proof runtime for generic execution environments.
//!
//! Proves the state transitions for a chunk of blocks.  Attests to chunk bounds
//! and execution input/output traces.

mod chunk;
mod chunk_processing;
mod private_inputs;
mod runtime;

pub use chunk::*;
pub use chunk_processing::*;
pub use private_inputs::*;
pub use runtime::*;
