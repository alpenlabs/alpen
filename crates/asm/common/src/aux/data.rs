//! Auxiliary data request and response types.
//!
//! Defines the types of auxiliary data that subprotocols can request during
//! the pre-processing phase, along with the response structures returned
//! to subprotocols after verification.

use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use strata_btc_types::RawBitcoinTx;

use crate::{AsmCompactMmr, AsmMerkleProof, Hash, L1TxIndex};

/// Collection of auxiliary data requests from subprotocols, organized by transaction index.
///
/// During pre-processing, subprotocols declare what auxiliary data they need for
/// each transaction. This structure aggregates all requests by type (manifest leaves,
/// Bitcoin transactions, etc.) to be fulfilled by external workers before the main
/// processing phase.
#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct AuxRequests {
    pub manifest_leaves: BTreeMap<L1TxIndex, ManifestLeavesRequest>,
    pub bitcoin_txs: BTreeMap<L1TxIndex, BitcoinTxRequest>,
}

/// Raw auxiliary data responses from workers, organized by transaction index.
///
/// This structure holds unverified auxiliary data (manifest leaves with proofs,
/// Bitcoin transactions) that will be verified by `AuxDataProvider` before being
/// served to subprotocols during transaction processing.
#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct AuxData {
    /// Map from transaction index to manifest leaves with proofs (unverified)
    pub manifest_leaves: BTreeMap<L1TxIndex, ManifestLeavesWithProofs>,
    /// Map from transaction index to Bitcoin transaction data
    pub bitcoin_txs: BTreeMap<L1TxIndex, RawBitcoinTx>,
}

/// Raw auxiliary data responses from workers, organized by transaction index.
///
/// This structure holds unverified auxiliary data (manifest leaves with proofs,
/// Bitcoin transactions) that will be verified by `AuxDataProvider` before being
/// served to subprotocols during transaction processing.
#[derive(Debug, Clone, Default, BorshSerialize, BorshDeserialize)]
pub struct AuxData2 {
    /// Map from transaction index to manifest leaves with proofs (unverified)
    pub manifest_leaves: Vec<(Hash, AsmMerkleProof)>,
    /// Map from transaction index to Bitcoin transaction data
    pub bitcoin_txs: Vec<RawBitcoinTx>,
}

/// Request for manifest leaves over an inclusive range.
///
/// Carries the compact manifest MMR snapshot so the provider can
/// expand it and verify the included MMR proofs for each leaf.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ManifestLeavesRequest {
    /// Starting L1 block height (inclusive)
    pub start_height: u64,
    /// Ending L1 block height (inclusive)
    pub end_height: u64,
    /// Compact manifest MMR snapshot used for proof verification
    pub manifest_mmr: AsmCompactMmr,
}

/// Request for a single Bitcoin transaction by txid.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct BitcoinTxRequest {
    /// The Bitcoin transaction ID to fetch (32 bytes)
    pub txid: [u8; 32],
}

/// Response containing manifest leaves for a contiguous block range.
///
/// This is returned to subprotocols by the `AuxDataProvider` after MMR proof
/// verification has succeeded. Only the leaves are included in this response.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ManifestLeavesResponse {
    /// One manifest hash per block in range, ordered by height
    pub leaves: Vec<Hash>,
}

/// Manifest leaves with their proofs for a contiguous block range.
///
/// This structure is provided to the `AuxDataProvider` by external workers and
/// is used solely for verification. It contains both the leaves and their
/// corresponding proofs, but inclusion here does not imply prior verification.
///
/// Note: For now we include a separate Merkle proof for each leaf. Since the
/// leaves are contiguous within the range, this could be optimized to use a
/// single proof (or a more compact multi-proof) covering all leaves.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ManifestLeavesWithProofs {
    /// One manifest hash per block in range, ordered by height
    pub leaves: Vec<Hash>,
    /// Per-leaf MMR proofs (same order as `leaves`)
    pub proofs: Vec<AsmMerkleProof>,
}
