//! Auxiliary data request specifications.
//!
//! Defines the types of auxiliary data that subprotocols can request during
//! the pre-processing phase.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmManifestCompactMmr;

/// Specification for auxiliary data needed by a transaction.
///
/// During `pre_process_txs`, subprotocols can register auxiliary data requirements
/// by creating `AuxRequestSpec` instances. The orchestration layer will then fulfill
/// these requests before the main processing phase begins.
///
/// For manifest leaf requests, the compact MMR snapshot is carried in the request
/// so the resolver can verify MMR proofs without separate context.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum AuxRequestSpec {
    /// Request manifest leaves from a range of L1 blocks (inclusive).
    ///
    /// A manifest leaf consists of the manifest hash and an MMR proof.
    ManifestLeaves(ManifestLeavesRequest),

    /// Request a specific Bitcoin transaction by its transaction ID.
    BitcoinTx(BitcoinTxRequest),
}

/// Request for manifest leaves over an inclusive range.
///
/// Carries the compact manifest MMR snapshot so the resolver can
/// expand it and verify the included MMR proofs for each leaf.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ManifestLeavesRequest {
    /// Starting L1 block height (inclusive)
    pub start_height: u64,
    /// Ending L1 block height (inclusive)
    pub end_height: u64,
    /// Compact manifest MMR snapshot used for proof verification
    pub manifest_mmr: AsmManifestCompactMmr,
}

/// Request for a single Bitcoin transaction by txid.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct BitcoinTxRequest {
    /// The Bitcoin transaction ID to fetch (32 bytes)
    pub txid: [u8; 32],
}

impl AuxRequestSpec {
    /// Creates a request for manifest leaves over a block range.
    ///
    /// # Arguments
    /// * `start_height` - Starting L1 block height (inclusive)
    /// * `end_height` - Ending L1 block height (inclusive)
    /// * `manifest_mmr` - Compact manifest MMR snapshot for verification
    pub fn manifest_leaves(
        start_height: u64,
        end_height: u64,
        manifest_mmr: AsmManifestCompactMmr,
    ) -> Self {
        Self::ManifestLeaves(ManifestLeavesRequest {
            start_height,
            end_height,
            manifest_mmr,
        })
    }

    /// Creates a request for a specific Bitcoin transaction.
    ///
    /// # Arguments
    /// * `txid` - The Bitcoin transaction ID to fetch (32 bytes)
    pub fn bitcoin_tx(txid: [u8; 32]) -> Self {
        Self::BitcoinTx(BitcoinTxRequest { txid })
    }
}
