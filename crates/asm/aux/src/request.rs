//! Auxiliary data request specifications.
//!
//! Defines the types of auxiliary data that subprotocols can request during
//! the pre-processing phase.

use borsh::{BorshDeserialize, BorshSerialize};

/// Specification for auxiliary data needed by a transaction.
///
/// During `pre_process_txs`, subprotocols can register auxiliary data requirements
/// by creating `AuxRequestSpec` instances. The orchestration layer will then fulfill
/// these requests before the main processing phase begins.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum AuxRequestSpec {
    /// Request manifest leaves from a range of L1 blocks (inclusive).
    ///
    /// A manifest leaf consists of:
    /// - The `AsmManifestHash` (the actual leaf value stored in the MMR)
    /// - An MMR proof showing that this hash is committed
    ///
    /// This is lightweight - it doesn't include the full manifest data
    /// (blkid, wtxids_root, logs), only the hash and proof.
    ManifestLeaves {
        /// Starting L1 block height (inclusive)
        start_height: u64,
        /// Ending L1 block height (inclusive)
        end_height: u64,
    },

    /// Request a specific Bitcoin transaction by its transaction ID.
    ///
    /// This returns the raw transaction bytes, which can then be parsed
    /// and validated by the requesting subprotocol.
    ///
    /// Example use case: Bridge subprotocol requesting deposit request
    /// transactions for validation (alternative to OP_RETURN data).
    ///
    /// Note: Txid is stored as 32 bytes since bitcoin::Txid doesn't implement Borsh traits.
    BitcoinTx {
        /// The Bitcoin transaction ID to fetch (32 bytes)
        txid: [u8; 32],
    },
}

impl AuxRequestSpec {
    /// Creates a request for manifest leaves over a block range.
    ///
    /// # Arguments
    /// * `start_height` - Starting L1 block height (inclusive)
    /// * `end_height` - Ending L1 block height (inclusive)
    pub fn manifest_leaves(start_height: u64, end_height: u64) -> Self {
        Self::ManifestLeaves {
            start_height,
            end_height,
        }
    }

    /// Creates a request for a specific Bitcoin transaction.
    ///
    /// # Arguments
    /// * `txid` - The Bitcoin transaction ID to fetch (32 bytes)
    pub fn bitcoin_tx(txid: [u8; 32]) -> Self {
        Self::BitcoinTx { txid }
    }
}
