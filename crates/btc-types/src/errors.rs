//! Errors during parsing/handling/conversion of Bitcoin types.

use bitcoin::{address, secp256k1, AddressType};
use thiserror::Error;
use strata_identifiers::Buf32;

/// Parsing errors that can occur with Bitcoin types,
/// such as addresses, pubkeys, and scripts.
#[derive(Debug, Clone, Error)]
pub enum ParseError {
    /// The provided pubkey is invalid.
    #[error("supplied pubkey is invalid")]
    InvalidPubkey(#[from] secp256k1::Error),

    /// The provided address is invalid.
    #[error("supplied address is invalid")]
    InvalidAddress(#[from] address::ParseError),

    /// The provided script is invalid.
    #[error("supplied script is invalid")]
    InvalidScript(#[from] address::FromScriptError),

    /// The provided 32-byte buffer is not a valid point on the curve.
    #[error("not a valid point on the curve: {0}")]
    InvalidPoint(Buf32),

    /// Converting from an unsupported [`Address`](bitcoin::Address) type for a [`Buf32`].
    #[error("only taproot addresses are supported but found {0:?}")]
    UnsupportedAddress(Option<AddressType>),

    /// Could not get a network address from descriptor
    /// Using String error as [`bitcoin_bosd::DescriptorError`] does not impl Clone
    #[error("descriptor: {0}")]
    Descriptor(String),
}

impl From<strata_identifiers::ParseError> for ParseError {
    fn from(value: strata_identifiers::ParseError) -> Self {
        match value {
            strata_identifiers::ParseError::InvalidPubkey(e) => Self::InvalidPubkey(e),
            strata_identifiers::ParseError::InvalidPoint(buf) => Self::InvalidPoint(buf),
            strata_identifiers::ParseError::UnsupportedAddress(ty) => Self::UnsupportedAddress(ty),
        }
    }
}
