//! SSZ (Simple Serialize) types for identifiers.
//!
//! This crate provides SSZ-serializable versions of types from `strata-identifiers`,
//! enabling efficient merkleization for zero-knowledge proofs and Ethereum compatibility.
//!
//! ## Features
//!
//! - SSZ serialization/deserialization
//! - Merkle tree generation via `tree_hash_root()`
//! - Bidirectional conversion with Borsh-based `strata-identifiers`
//!
//! ## Example
//!
//! ```rust,ignore
//! use strata_identifiers_ssz::L1BlockCommitment;
//! use ssz::Encode;
//! use tree_hash::TreeHash;
//!
//! let commitment = L1BlockCommitment {
//!     height: 100,
//!     blkid: [0u8; 32].into(),
//! };
//!
//! // SSZ serialization
//! let bytes = commitment.as_ssz_bytes();
//!
//! // Merkle root
//! let root = commitment.tree_hash_root();
//! ```

#![warn(missing_debug_implementations)]
#![warn(missing_docs)]
#![allow(
    unused_crate_dependencies,
    reason = "build dependencies are not used in the source code"
)]

#[expect(
    missing_debug_implementations,
    reason = "generated code does not implement Debug"
)]
#[expect(missing_docs, reason = "generated code is not documented")]
#[expect(unused_imports, reason = "generated code may have unused imports")]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/generated_ssz.rs"));
}

pub use generated::*;
pub use ssz::{Decode, Encode};
pub use tree_hash::TreeHash;
