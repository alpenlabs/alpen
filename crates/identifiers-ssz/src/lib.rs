//! SSZ (Simple Serialize) types for identifiers.

#![warn(missing_debug_implementations)]
#![warn(missing_docs)]
#![allow(unused_crate_dependencies)]

include!(concat!(env!("OUT_DIR"), "/generated_ssz.rs"));

pub use ssz::{Decode, Encode};
pub use tree_hash::TreeHash;
