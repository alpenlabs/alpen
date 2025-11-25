//! Error types for threshold signing operations.

use std::fmt;

/// Errors that can occur during threshold signing operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThresholdSigningError {
    /// Not enough signatures to meet the threshold.
    InsufficientSignatures { provided: usize, required: usize },
    /// Invalid public key data.
    InvalidPublicKey { index: usize, reason: String },
    /// Invalid threshold value.
    InvalidThreshold { threshold: u8, total_keys: usize },
    /// Threshold cannot be zero.
    ZeroThreshold,
    /// Signature verification failed.
    InvalidSignature { index: u8 },
    /// Invalid signature format.
    InvalidSignatureFormat,
    /// Duplicate signer index in signature set.
    DuplicateSignerIndex(u8),
    /// Signer index out of bounds.
    SignerIndexOutOfBounds { index: u8, max: usize },
    /// Member already exists in the configuration.
    MemberAlreadyExists,
    /// Duplicate member in add list.
    DuplicateAddMember,
    /// Duplicate member in remove list.
    DuplicateRemoveMember,
    /// Member not found in the configuration.
    MemberNotFound,
    /// Invalid message hash.
    InvalidMessageHash,
}

impl fmt::Display for ThresholdSigningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientSignatures { provided, required } => {
                write!(
                    f,
                    "insufficient signatures: provided {}, required {}",
                    provided, required
                )
            }
            Self::InvalidPublicKey { index, reason } => {
                write!(f, "invalid public key at index {}: {}", index, reason)
            }
            Self::InvalidThreshold {
                threshold,
                total_keys,
            } => {
                write!(
                    f,
                    "invalid threshold: {} exceeds total keys {}",
                    threshold, total_keys
                )
            }
            Self::ZeroThreshold => write!(f, "threshold cannot be zero"),
            Self::InvalidSignature { index } => {
                write!(f, "invalid signature at index {}", index)
            }
            Self::InvalidSignatureFormat => write!(f, "invalid signature format"),
            Self::DuplicateSignerIndex(index) => {
                write!(f, "duplicate signer index: {}", index)
            }
            Self::SignerIndexOutOfBounds { index, max } => {
                write!(f, "signer index {} out of bounds (max: {})", index, max)
            }
            Self::MemberAlreadyExists => write!(f, "member already exists"),
            Self::DuplicateAddMember => write!(f, "duplicate member in add list"),
            Self::DuplicateRemoveMember => write!(f, "duplicate member in remove list"),
            Self::MemberNotFound => write!(f, "member not found"),
            Self::InvalidMessageHash => write!(f, "invalid message hash"),
        }
    }
}

impl std::error::Error for ThresholdSigningError {}

impl From<secp256k1::Error> for ThresholdSigningError {
    fn from(e: secp256k1::Error) -> Self {
        Self::InvalidPublicKey {
            index: 0,
            reason: e.to_string(),
        }
    }
}
