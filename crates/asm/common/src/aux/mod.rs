//! Auxiliary input framework for the Anchor State Machine (ASM).
//!
//! This crate provides infrastructure for subprotocols to request and receive
//! auxiliary data during ASM state transitions. The framework consists of:
//!
//! - **Request Phase** ([`pre_process_txs`]): Subprotocols use [`AuxRequestCollector`] to declare
//!   what auxiliary data they need, keyed by transaction index.
//!
//! - **Fulfillment Phase**: External workers fetch the requested data and produce responses for
//!   each request type (e.g., manifest leaves with proofs, raw Bitcoin txs).
//!
//! - **Processing Phase** ([`process_txs`]): Subprotocols use [`AuxDataProvider`] to access the
//!   fulfilled auxiliary data. The provider verifies data based on each request.
//!
//! ## Supported Request Types
//!
//! - **Manifest Leaves**: Fetch manifest hashes and MMR proofs for a range of L1 blocks
//!   (lightweight - doesn't include full manifest data). The request must include the
//!   `AsmManifestCompactMmr` snapshot for verifying the MMR proofs in the response.
//!
//! - **Bitcoin Transactions**: Fetch raw Bitcoin transaction data by txid (for bridge subprotocol
//!   validation). The request must include the expected txid to verify against the response.
//!
//! ## Request Granularity
//!
//! Requests are keyed by **transaction index** (`L1TxIndex`) within an L1 block.
//! Each transaction can request at most one item per request type (e.g., one
//! manifest-leaves request and one bitcoin-tx request). Not all transactions
//! need to request auxiliary data.
//!
//! ## Verification
//!
//! Each request type must include all necessary information to verify its response.
//! The [`AuxDataProvider`] validates responses using request-provided verification data
//! before returning them to subprotocols, ensuring all auxiliary data is cryptographically
//! sound.
//!
//! **IMPORTANT**: For a given transaction index and request type, there must be
//! at most one response. If your use case requires multiple pieces of auxiliary
//! data for a single transaction, define a request type that bundles the needed
//! data together.
mod collector;
mod data;
mod errors;
mod provider;

// Re-export main types
pub use collector::AuxRequestCollector;
pub use data::{
    AuxData, AuxRequests, BitcoinTxRequest, ManifestLeavesRequest, ManifestLeavesResponse,
    ManifestLeavesWithProofs,
};
pub use errors::{AuxError, AuxResult, BitcoinTxError, ManifestLeavesError};
pub use provider::AuxDataProvider;
