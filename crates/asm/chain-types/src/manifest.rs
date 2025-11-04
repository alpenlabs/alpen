use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::{Buf32, hash::compute_borsh_hash};

/// ASM execution manifest containing encoded logs and header data.
///
/// This type represents the output of ASM execution and contains encoded data
/// that doesn't require decoding Bitcoin types, making it suitable for use in
/// contexts that cannot depend on the bitcoin crate (e.g., ledger-types).
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AsmManifest {
    /// Bitcoin block hash (32-byte double SHA256 of the block header).
    pub block_root: Buf32,

    /// Merkle root of the block's witness transaction IDs.
    pub wtx_root: Buf32,

    /// Logs emitted by the ASM STF.
    pub logs: Vec<AsmLog>,
}

/// Encoded ASM log entry.
#[derive(Clone, Debug, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct AsmLog(Vec<u8>);

impl AsmLog {
    /// Create an AsmLog from raw bytes.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Get a reference to the raw log bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consume the log and return the raw bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl From<Vec<u8>> for AsmLog {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl AsmManifest {
    pub fn new(block_root: Buf32, wtx_root: Buf32, logs: Vec<AsmLog>) -> Self {
        Self {
            block_root,
            wtx_root,
            logs,
        }
    }

    pub fn block_root(&self) -> &Buf32 {
        &self.block_root
    }

    pub fn wtx_root(&self) -> &Buf32 {
        &self.wtx_root
    }

    pub fn logs(&self) -> &[AsmLog] {
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
