//! Auxiliary request and response data.
//!
//! Defines the types of auxiliary data that subprotocols can request during
//! the pre-processing phase, along with the response structures returned
//! to subprotocols after verification.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_btc_types::RawBitcoinTx;
use strata_identifiers::Buf32;

use crate::{AsmMerkleProof, Hash32};

/// Collection of auxiliary data requests from subprotocols.
///
/// During pre-processing, subprotocols declare what auxiliary data they need.
/// External workers fulfill that before the main processing phase.
#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct AuxRequests {
    /// Requested manifest leaf height ranges.
    pub manifest_leaves: Vec<ManifestLeafRange>,

    /// [Txid](bitcoin::Txid) of the requested transactions.
    // NOTE: Using Buf32 here instead of Txid because of borsh serialization requirement
    pub bitcoin_txs: Vec<Buf32>,
}

/// Collection of auxiliary data responses for subprotocols.
///
/// Contains unverified Bitcoin transactions and manifest leaves returned by external workers.
/// This data must be validated before use during the main processing phase.
#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct AuxData {
    /// Manifest leaves with their MMR proofs
    pub manifest_leaves: Vec<ManifestLeafWithProof>,
    /// Raw Bitcoin transaction data (unverified)
    pub bitcoin_txs: Vec<RawBitcoinTx>,
}

/// Manifest leaf height range (inclusive).
#[derive(Debug, Clone, Copy, BorshSerialize, BorshDeserialize)]
pub struct ManifestLeafRange {
    /// Start height (inclusive)
    pub start_height: u64,
    /// End height (inclusive)
    pub end_height: u64,
}

/// Manifest leaf with its MMR proof.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct ManifestLeafWithProof {
    /// The manifest leaf hash
    pub leaf: Hash32,
    /// The MMR proof for this leaf
    pub proof: AsmMerkleProof,
}
