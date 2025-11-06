use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, L1BlockId, hash::compute_borsh_hash};

use crate::{AsmLogEntry, Hash};

/// The manifest output produced after processing an L1 block.
///
/// This structure represents the result of parsing and validating an L1 (Bitcoin) block,
/// containing the essential commitments and execution logs needed.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AsmManifest {
    /// The L1 block identifier, essentially a [`bitcoin::BlockHash`].
    pub blkid: L1BlockId,

    /// The witness transaction ID merkle root, essentially a [`bitcoin::WitnessMerkleNode`].
    ///
    /// Used instead of [`bitcoin::TxMerkleNode`] to include witness data for complete transaction
    /// verification and malleability protection.
    pub wtxids_root: Buf32,

    /// Ordered list of log entries emitted by different subprotocols during L1 block processing.
    pub logs: Vec<AsmLogEntry>,
}

impl AsmManifest {
    /// Creates a new ASM manifest.
    pub fn new(blkid: L1BlockId, wtxids_root: Buf32, logs: Vec<AsmLogEntry>) -> Self {
        Self {
            blkid,
            wtxids_root,
            logs,
        }
    }

    /// Returns the L1 block identifier.
    pub fn blkid(&self) -> &L1BlockId {
        &self.blkid
    }

    /// Returns the witness transaction ID merkle root.
    pub fn wtxids_root(&self) -> &Buf32 {
        &self.wtxids_root
    }

    /// Returns the log entries.
    pub fn logs(&self) -> &[AsmLogEntry] {
        &self.logs
    }

    /// Computes the hash of the manifest.
    ///
    /// **TODO: PG**: This should use SSZ to compute the root of the `AsmManifest` container. SSZ
    /// would enable creating Merkle inclusion proofs for individual fields (logs,
    /// `wtxids_root`, etc.) when needed. Currently uses Borsh serialization.
    pub fn compute_hash(&self) -> Hash {
        compute_borsh_hash(&self).into()
    }
}
