//! Auxiliary input framework for the Anchor State Machine (ASM).
//!
//! This module provides infrastructure for subprotocols to request and receive
//! auxiliary data during ASM state transitions. The framework consists of:
//!
//! - **Request Phase** ([`pre_process_txs`]): Subprotocols use [`AuxRequestCollector`] to declare
//!   what auxiliary data they need.
//!
//! - **Fulfillment Phase**: External workers fetch the requested data and produce [`AuxData`]
//!   containing manifest hashes with MMR proofs and raw Bitcoin transactions.
//!
//! - **Processing Phase** ([`process_txs`]): Subprotocols use [`VerifiedAuxData`] to access the
//!   verified auxiliary data. The struct verifies all data upfront during construction.
//!
//! ## Supported Auxiliary Data Types
//!
//! - **Manifest Hashes**: Hashes of [`AsmManifest`](crate::AsmManifest) with MMR proofs for ranges
//!   of L1 blocks. The verified data verifies MMR proofs against the compact MMR snapshot.
//!
//! - **Bitcoin Transactions**: Raw Bitcoin transaction data by txid (for bridge subprotocol
//!   validation). The verified data decodes and indexes transactions by their txid.
mod collector;
mod data;
mod errors;
mod provider;

// Re-export main types
pub use collector::AuxRequestCollector;
pub use data::{AuxData, AuxRequests, ManifestHashRange, VerifiableManifestHash};
pub use errors::{AuxError, AuxResult};
pub use provider::VerifiedAuxData;
