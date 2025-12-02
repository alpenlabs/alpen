//! Error types for threshold signature operations.

use thiserror::Error;

/// Errors that can occur during threshold signature operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ThresholdSignatureError {
    /// Not enough signatures to meet the threshold.
    #[error("insufficient signatures: provided {provided}, required {required}")]
    InsufficientSignatures { provided: usize, required: usize },

    /// Invalid public key data.
    #[error("invalid public key{}: {reason}", index.map(|i| format!(" at index {}", i)).unwrap_or_default())]
    InvalidPublicKey {
        index: Option<usize>,
        reason: String,
    },

    /// Invalid threshold value.
    #[error("invalid threshold: {threshold} exceeds total keys {total_keys}")]
    InvalidThreshold { threshold: u8, total_keys: usize },

    /// Signature verification failed.
    #[error("invalid signature at index {index}")]
    InvalidSignature { index: u8 },

    /// Invalid signature format.
    #[error("invalid signature format")]
    InvalidSignatureFormat,

    /// Duplicate signer index in signature set.
    #[error("duplicate signer index: {0}")]
    DuplicateSignerIndex(u8),

    /// Signer index out of bounds.
    #[error("signer index {index} out of bounds (max: {max})")]
    SignerIndexOutOfBounds { index: u8, max: usize },

    /// Member already exists in the configuration.
    #[error("member already exists")]
    MemberAlreadyExists,

    /// Duplicate member in add list.
    #[error("duplicate member in add list")]
    DuplicateAddMember,

    /// Duplicate member in remove list.
    #[error("duplicate member in remove list")]
    DuplicateRemoveMember,

    /// Member not found in the configuration.
    #[error("member not found")]
    MemberNotFound,

    /// Invalid message hash.
    #[error("invalid message hash")]
    InvalidMessageHash,
}

impl From<secp256k1::Error> for ThresholdSignatureError {
    fn from(e: secp256k1::Error) -> Self {
        Self::InvalidPublicKey {
            index: None,
            reason: e.to_string(),
        }
    }
}
