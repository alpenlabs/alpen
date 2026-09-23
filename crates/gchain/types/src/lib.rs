//! Traits describing a generic chain and the processor stages run over it.
//!
//! Chain history is modelled as a directed acyclic graph: nodes are states "at
//! rest" and links are the transitions between them, so that block sync and
//! checkpoint sync are two paths through the same structure rather than two
//! separate mechanisms.

#![expect(missing_debug_implementations, reason = "clippy is wrong")]
#![expect(
    type_alias_bounds,
    reason = "bounds enforced by type aliases using associated types"
)]

mod chain_provider;
mod chain_spec;
mod errors;
mod processor;
mod processor_tracking;
mod version;

pub use chain_provider::*;
pub use chain_spec::*;
pub use errors::*;
pub use processor::*;
pub use processor_tracking::*;
pub use version::*;
