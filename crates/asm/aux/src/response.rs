//! Auxiliary data response types.
//!
//! Defines the structures that workers use to fulfill auxiliary data requests.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::AsmManifestHash;

use crate::types::ManifestMmrProof;

/// Auxiliary data response fulfilling a specific request.
///
/// Workers create these envelopes to provide the data requested by
/// subprotocols during pre-processing. Each envelope corresponds to
/// one `AuxRequestSpec`.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub enum AuxResponseEnvelope {
    /// Manifest leaves (hash + proof) for a requested block range.
    ///
    /// This is the response to `AuxRequestSpec::ManifestLeaves`.
    /// Contains the manifest hashes and proofs for each block in the range.
    ManifestLeaves(ManifestLeaves),

    /// Raw Bitcoin transaction bytes.
    ///
    /// This is the response to `AuxRequestSpec::BitcoinTx`.
    /// Contains the full serialized transaction data.
    ///
    /// Note: Txid is stored as 32 bytes since bitcoin::Txid doesn't implement Borsh traits.
    BitcoinTx(Vec<u8>),
}

/// Aggregated manifest leaves data for a contiguous block range.
///
/// Note: For now we include a separate Merkle proof for each leaf. Since the
/// leaves are contiguous within the range, this could be optimized to use a
/// single proof (or a more compact multi-proof) covering all leaves.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct ManifestLeaves {
    /// One manifest hash per block in range, ordered by height
    pub leaves: Vec<AsmManifestHash>,
    /// Per-leaf MMR proofs (same order as `leaves`)
    pub proofs: Vec<ManifestMmrProof>,
}

impl AuxResponseEnvelope {
    /// Returns the variant name as a string (for error messages).
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::ManifestLeaves(_) => "ManifestLeaves",
            Self::BitcoinTx(_) => "BitcoinTx",
        }
    }
}
