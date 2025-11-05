use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, L1BlockId, hash::compute_borsh_hash};

use crate::AsmLogEntry;

/// ASM execution manifest containing encoded logs and header data.
///
/// This type represents the output of ASM execution and contains encoded data
/// that doesn't require decoding Bitcoin types, making it suitable for use in
/// contexts that cannot depend on the bitcoin crate (e.g., ledger-types).
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AsmManifest {
    /// Bitcoin block hash (32-byte double SHA256 of the block header).
    pub block_root: L1BlockId,

    /// Merkle root of the block's witness transaction IDs.
    pub wtx_root: Buf32,

    /// Logs emitted by the ASM STF.
    pub logs: Vec<AsmLogEntry>,
}

impl AsmManifest {
    pub fn new(block_root: L1BlockId, wtx_root: Buf32, logs: Vec<AsmLogEntry>) -> Self {
        Self {
            block_root,
            wtx_root,
            logs,
        }
    }

    pub fn block_root(&self) -> &L1BlockId {
        &self.block_root
    }

    pub fn wtx_root(&self) -> &Buf32 {
        &self.wtx_root
    }

    pub fn logs(&self) -> &[AsmLogEntry] {
        &self.logs
    }

    /// Computes the flat SHA-256 hash of the Borsh-encoded manifest.
    ///
    /// This serves as a temporary stand-in for the SSZ hash tree root that will
    /// ultimately back the header MMR leaf.
    pub fn compute_hash(&self) -> [u8; 32] {
        compute_borsh_hash(self).into()
    }
}
