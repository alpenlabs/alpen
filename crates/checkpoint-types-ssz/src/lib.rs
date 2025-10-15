//! SSZ (Simple Serialize) types for checkpoint types.
//!
//! This crate provides SSZ-serializable versions of types from `strata-checkpoint-types`,
//! enabling efficient merkleization for zero-knowledge proofs and Ethereum compatibility.

#![warn(missing_debug_implementations)]
#![warn(missing_docs)]
#![allow(
    unused_crate_dependencies,
    reason = "build dependencies are not used in the source code"
)]

#[allow(
    missing_debug_implementations,
    missing_docs,
    unused_imports,
    reason = "generated code may not follow all lint rules"
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated_ssz.rs"));
}

#[allow(unused_imports, reason = "re-exported types may not be used in tests")]
pub use generated::*;
pub use ssz::{Decode, Encode};
pub use tree_hash::TreeHash;
