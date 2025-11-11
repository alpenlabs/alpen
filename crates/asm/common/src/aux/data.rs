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
    /// Requested manifest leaf height ranges as (start_height, end_height) inclusive.
    pub manifest_leaves: Vec<(u64, u64)>,

    /// [Txid](bitcoin::Txid) of the requested transactions.
    // NOTE: Using Buf32 here instead of Txid because of borsh serialization requirement
    pub bitcoin_txs: Vec<Buf32>,
}

/// Auxiliary data containing unverified Bitcoin transactions and manifest leaves.
///
/// This structure holds auxiliary data in vector form for efficient batch processing.
/// The data is unverified and must be validated before use, typically by passing it
/// to [`AuxDataProvider::try_new`] which verifies all proofs and decodes transactions.
#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct AuxData {
    /// Manifest leaves with their MMR proofs
    pub manifest_leaves: Vec<(Hash32, AsmMerkleProof)>,
    /// Raw Bitcoin transaction data (unverified)
    pub bitcoin_txs: Vec<RawBitcoinTx>,
}
