//! Auxiliary input framework for the Anchor State Machine (ASM).
//!
//! This module provides infrastructure for subprotocols to request and receive
//! auxiliary data during ASM state transitions. The framework consists of:
//!
//! - **Request Phase** ([`pre_process_txs`]): Subprotocols use [`AuxRequestCollector`] to declare
//!   what auxiliary data they need.
//!
//! - **Fulfillment Phase**: External workers fetch the requested data and produce [`AuxData`]
//!   containing manifest leaves with MMR proofs and raw Bitcoin transactions.
//!
//! - **Processing Phase** ([`process_txs`]): Subprotocols use [`AuxDataProvider`] to access the
//!   verified auxiliary data. The provider verifies all data upfront during construction.
//!
//! ## Supported Auxiliary Data Types
//!
//! - **Manifest Leaves**: Manifest hashes with MMR proofs for ranges of L1 blocks.
//!   The provider verifies MMR proofs against the compact MMR snapshot.
//!
//! - **Bitcoin Transactions**: Raw Bitcoin transaction data by txid (for bridge subprotocol
//!   validation). The provider decodes and indexes transactions by their txid.
//!
//! ## Verification
//!
//! The [`AuxDataProvider`] performs all verification during construction:
//! - Decodes all Bitcoin transactions and verifies they match their txids
//! - Verifies all MMR proofs for manifest leaves
//! - Indexes verified data for efficient lookup during transaction processing
//!
//! This upfront verification ensures all auxiliary data is cryptographically sound
//! before any subprotocol accesses it.
mod collector;
mod data;
mod errors;
mod provider;

// Re-export main types
pub use collector::AuxRequestCollector;
pub use data::{AuxData, AuxRequests};
pub use errors::{AuxError, AuxResult};
pub use provider::AuxDataProvider;
