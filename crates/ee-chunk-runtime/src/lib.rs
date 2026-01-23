//! Chunk proof runtime for generic execution environments.
//!
//! Proves the state transitions for a chunk of blocks.  Attests to chunk bounds
//! and execution input/output traces.

// TODO remove these
extern crate strata_acct_types as _;
extern crate strata_snark_acct_types as _;

mod chunk;
mod chunk_processing;
mod private_inputs;

pub use chunk::*;
pub use chunk_processing::*;
pub use private_inputs::*;
