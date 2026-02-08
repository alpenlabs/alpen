//! Strata common EE chain types.
//!
//! This is primarily at the boundary between the internal EE account state and
//! the execution env chain.  These are not generally involved in the
//! orchestration layer protocol.
// @jose: fineeeee

#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

// Publicly re-export only the SSZ items this crate's API intends to expose
pub use ssz_generated::ssz::{block::*, chunk::*, io::*};

mod block;
mod chunk;
mod io;
mod sequence_tracker;

pub use sequence_tracker::{SeqError, SeqResult, SequenceTracker};
