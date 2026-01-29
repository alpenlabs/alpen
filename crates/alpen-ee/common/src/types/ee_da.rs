//! EE Data Availability types and constants.
//!
//! This module defines the manifest for tracking chunked DA publications
//! and related constants for the EE DA subprotocol.

use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════════════════
// EE DA Constants
// ═══════════════════════════════════════════════════════════════════════════════

/// OP_RETURN tag for EE DA transactions.
pub const EE_DA_TAG: [u8; 4] = *b"EEDA";

/// Subprotocol ID for EE DA (per SPS-63).
pub const EE_DA_SUBPROTOCOL_ID: u8 = 3;

/// Transaction type byte for DA inscriptions.
pub const EE_DA_TX_TYPE: u8 = 0;

/// Maximum chunk payload size in bytes.
///
/// Matches the value in btcio chunked_envelope module.
pub const EE_DA_MAX_CHUNK_PAYLOAD: usize = 330_000;

/// Maximum total payload size (10MB).
pub const EE_DA_MAX_PAYLOAD_SIZE: usize = 10 * 1024 * 1024;

/// Maximum number of chunks per payload.
pub const EE_DA_MAX_CHUNKS: u16 = 31;

/// Minimum confirmations required for DA to be considered published.
pub const MIN_DA_CONFIRMATIONS: u32 = 1;

/// Default number of confirmations required for DA to be considered finalized.
pub const DEFAULT_DA_FINALIZATION_DEPTH: u32 = 6;

// ═══════════════════════════════════════════════════════════════════════════════
// Manifest Types
// ═══════════════════════════════════════════════════════════════════════════════

/// Manifest for tracking chunked DA publications.
///
/// This manifest is created after a chunked blob submission and can be used
/// by verifiers to reassemble and validate the original payload from L1.
///
/// The manifest is serialized using bincode for compact storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EeDaBlobManifest {
    /// Version byte (0 for v0).
    pub version: u8,
    /// SHA256 hash of the full payload.
    pub blob_hash: [u8; 32],
    /// Original payload size in bytes.
    pub blob_size: u64,
    /// Total number of chunks.
    pub total_chunks: u16,
    /// Wtxids of the reveal transactions (ordered by chunk index).
    pub chunk_wtxids: Vec<[u8; 32]>,
}

impl EeDaBlobManifest {
    /// Creates a new manifest.
    pub fn new(
        blob_hash: [u8; 32],
        blob_size: u64,
        chunk_wtxids: Vec<[u8; 32]>,
    ) -> Result<Self, &'static str> {
        let total_chunks = chunk_wtxids.len() as u16;
        if total_chunks == 0 {
            return Err("manifest must have at least one chunk");
        }
        if total_chunks > EE_DA_MAX_CHUNKS {
            return Err("too many chunks in manifest");
        }

        Ok(Self {
            version: 0,
            blob_hash,
            blob_size,
            total_chunks,
            chunk_wtxids,
        })
    }

    /// Returns the last chunk's wtxid (for cross-blob linking).
    pub fn last_chunk_wtxid(&self) -> Option<&[u8; 32]> {
        self.chunk_wtxids.last()
    }

    /// Encodes the manifest to bytes using bincode.
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serialize(self).expect("manifest serialization should not fail")
    }

    /// Decodes a manifest from bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// DA Error Types
// ═══════════════════════════════════════════════════════════════════════════════

/// Errors that can occur during DA operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum DaError {
    /// Failed to submit DA.
    #[error("DA submit failed: {0}")]
    SubmitFailed(String),

    /// Failed to check DA status.
    #[error("DA status check failed: {0}")]
    StatusFailed(String),

    /// DA publication failed.
    #[error("DA publication failed: {0}")]
    PublishFailed(String),

    /// Unknown blob.
    #[error("unknown blob: {0}")]
    UnknownBlob(String),

    /// Payload too large.
    #[error("payload too large: {size} bytes exceeds maximum of {max} bytes")]
    PayloadTooLarge {
        /// Actual payload size.
        size: usize,
        /// Maximum allowed size.
        max: usize,
    },

    /// Internal error.
    #[error("internal error: {0}")]
    Internal(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_roundtrip() {
        let manifest = EeDaBlobManifest::new(
            [0x42; 32],
            1000,
            vec![[0x01; 32], [0x02; 32], [0x03; 32]],
        )
        .unwrap();

        // Encode to bytes
        let encoded = manifest.to_bytes();

        // Decode from bytes
        let decoded = EeDaBlobManifest::from_bytes(&encoded).unwrap();

        assert_eq!(manifest, decoded);
        assert_eq!(decoded.version, 0);
        assert_eq!(decoded.blob_hash, [0x42; 32]);
        assert_eq!(decoded.blob_size, 1000);
        assert_eq!(decoded.total_chunks, 3);
        assert_eq!(decoded.chunk_wtxids.len(), 3);
    }

    #[test]
    fn test_manifest_validation() {
        // Empty chunks should fail
        assert!(EeDaBlobManifest::new([0; 32], 0, vec![]).is_err());

        // Single chunk is valid
        assert!(EeDaBlobManifest::new([0; 32], 100, vec![[0; 32]]).is_ok());

        // Max chunks (31) should be valid
        let max_chunks: Vec<[u8; 32]> = (0..31).map(|i| [i as u8; 32]).collect();
        assert!(EeDaBlobManifest::new([0; 32], 10_000_000, max_chunks).is_ok());
    }

    #[test]
    fn test_last_chunk_wtxid() {
        let manifest = EeDaBlobManifest::new(
            [0; 32],
            1000,
            vec![[0x01; 32], [0x02; 32], [0x03; 32]],
        )
        .unwrap();

        assert_eq!(manifest.last_chunk_wtxid(), Some(&[0x03; 32]));
    }
}
