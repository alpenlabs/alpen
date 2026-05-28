//! Shared EE data-availability types and primitives.
//!
//! Lightweight crate holding the DA codec types (`DaBlob`, `EvmHeaderSummary`),
//! format constants, Bitcoin commit/reveal parsing helpers, and Bitcoin merkle
//! primitives that are needed by both the host-side witness builder and the
//! guest-side proof verifier. Kept free of Reth node, Alloy RPC, and async
//! Bitcoin-RPC dependencies so it can be linked from ZKVM guest builds.

pub mod blob;
pub mod merkle;
pub mod parse;

pub use blob::{reassemble_da_blob, DaBlob, EvmHeaderSummary, DA_BLOB_VERSION, EE_DA_MAGIC_BYTES};
pub use merkle::{
    bitcoin_hash_pair, bitcoin_inclusion_proof, bitcoin_merkle_root,
    bitcoin_merkle_root_from_leaves,
};
pub use parse::{
    commit_marker_payload, extract_da_chunks, extract_reveal_chunk, last_commit_reveal_vout,
    DaParseError,
};
