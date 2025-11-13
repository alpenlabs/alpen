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
    /// Requested manifest hash height ranges.
    pub manifest_hashes: Vec<ManifestHashRange>,

    /// [Txid](bitcoin::Txid) of the requested transactions.
    // NOTE: Using Buf32 here instead of Txid because of borsh serialization requirement
    pub bitcoin_txs: Vec<Buf32>,
}

/// Collection of auxiliary data responses for subprotocols.
///
/// Contains unverified Bitcoin transactions and manifest hashes returned by external workers.
/// This data must be validated before use during the main processing phase.
#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct AuxData {
    /// Manifest hashes with their MMR proofs (unverified)
    pub manifest_hashes: Vec<VerifiableManifestHash>,
    /// Raw Bitcoin transaction data (unverified)
    pub bitcoin_txs: Vec<RawBitcoinTx>,
}

/// Manifest hash height range (inclusive).
///
/// Represents a range of L1 block heights for which manifest hashes are requested.
#[derive(Debug, Clone, Copy, BorshSerialize, BorshDeserialize)]
pub struct ManifestHashRange {
    /// Start height (inclusive)
    pub start_height: u64,
    /// End height (inclusive)
    pub end_height: u64,
}

/// Manifest hash with its MMR proof.
///
/// Contains a hash of an [`AsmManifest`](crate::AsmManifest) along with an MMR proof
/// that can be used to verify the hash's inclusion in the manifest MMR at a specific position.
///
/// This is unverified data - the proof must be verified against a trusted compact MMR
/// before the hash can be considered valid.
#[derive(Debug, Clone, BorshSerialize, BorshDeserialize)]
pub struct VerifiableManifestHash {
    /// The hash of an [`AsmManifest`](crate::AsmManifest)
    pub hash: Hash32,
    /// The MMR proof for this manifest hash
    pub proof: AsmMerkleProof,
}
