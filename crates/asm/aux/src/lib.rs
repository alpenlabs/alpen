//! Auxiliary input framework for the Anchor State Machine (ASM).
//!
//! This crate provides infrastructure for subprotocols to request and receive
//! auxiliary data during ASM state transitions. The framework consists of:
//!
//! - **Request Phase** ([`pre_process_txs`]): Subprotocols use [`AuxRequestCollector`] to declare
//!   what auxiliary data they need, keyed by transaction index.
//!
//! - **Fulfillment Phase**: External workers (orchestration layer) fetch the requested data and
//!   produce responses for each request type (e.g., manifest leaves with proofs, raw Bitcoin txs).
//!
//! - **Resolution Phase** ([`process_txs`]): Subprotocols use [`AuxResolver`] to access the
//!   fulfilled auxiliary data. The resolver automatically verifies MMR proofs.
//!
//! # Design
//!
//! ## MMR Structure
//!
//! The manifest MMR stores one leaf per L1 block:
//! ```text
//! MMR Leaf = AsmManifestHash = Hash(AsmManifest)
//! ```
//!
//! Where `AsmManifest` contains:
//! - `blkid`: L1 block identifier
//! - `wtxids_root`: Witness transaction IDs merkle root
//! - `logs`: Vector of ASM log entries
//!
//! ## Request Granularity
//!
//! Requests are keyed by **transaction index** (`L1TxIndex`) within an L1 block.
//! Each transaction can request at most one item per request type (e.g., one
//! manifest-leaves request and one bitcoin-tx request). Not all transactions
//! need to request auxiliary data.
//!
//! **IMPORTANT**: For a given transaction index and request type, there must be
//! at most one response. If your use case requires multiple pieces of auxiliary
//! data for a single transaction, define a request type that bundles the needed
//! data together.
//!
//! ## Supported Request Types
//!
//! - **Manifest Leaves**: Fetch manifest hashes and MMR proofs for a range of L1 blocks
//!   (lightweight - doesn't include full manifest data)
//!
//! - **Bitcoin Transactions**: Fetch raw Bitcoin transaction data by txid (for bridge subprotocol
//!   validation)
//!
//! # Example Usage
//!
//! ```ignore
//! use strata_asm_aux::{AuxRequestCollector, AuxResolver};
//!
//! // During pre_process_txs:
//! fn pre_process_txs(
//!     state: &Self::State,
//!     txs: &[TxInputRef],
//!     collector: &mut AuxRequestCollector,
//!     anchor_pre: &AnchorState,
//!     params: &Self::Params,
//! ) {
//!     for (idx, tx) in txs.iter().enumerate() {
//!         // Request manifest leaves for L1 blocks 100-200
//!         // Include the manifest MMR snapshot for verification
//!         let mmr_compact = /* obtain from state */ todo!("compact MMR");
//!         let req = ManifestLeavesRequest { start_height: 100, end_height: 200, manifest_mmr: mmr_compact };
//!         collector.request_manifest_leaves(idx, req);
//!     }
//! }
//!
//! // During process_txs:
//! fn process_txs(
//!     state: &mut Self::State,
//!     txs: &[TxInputRef],
//!     anchor_pre: &AnchorState,
//!     aux_resolver: &AuxResolver,
//!     relayer: &mut impl MsgRelayer,
//!     params: &Self::Params,
//! ) {
//!     for (idx, tx) in txs.iter().enumerate() {
//!         // Get verified manifest leaves for a known range
//!         let mmr_compact = /* obtain from state */ todo!("compact MMR");
//!         let req = ManifestLeavesRequest { start_height: 100, end_height: 200, manifest_mmr: mmr_compact };
//!         let data = aux_resolver.get_manifest_leaves(idx, &req)?;
//!
//!         for hash in &data.leaves {
//!             // Use the verified manifest hash
//!             let _ = hash;
//!             // ... process
//!         }
//!     }
//! }
//! ```
//!
//! [`pre_process_txs`]: strata_asm_common::Subprotocol::pre_process_txs
//! [`process_txs`]: strata_asm_common::Subprotocol::process_txs

mod collector;
mod data;
mod error;
mod resolver;
mod types;

// Re-export main types
pub use collector::AuxRequestCollector;
pub use data::{
    BitcoinTxRequest, ManifestLeavesRequest, ManifestLeavesResponse, ManifestLeavesWithProofs,
};
pub use error::{AuxError, AuxResult};
pub use resolver::AuxResolver;
pub use types::{L1TxIndex, ManifestMmrProof};
