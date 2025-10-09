//! Errors during parsing/handling/conversion of identifiers.

#[cfg(feature = "fullbtc")]
use bitcoin::{AddressType, secp256k1};
use thiserror::Error;

use crate::buf::Buf32;

/// Parsing errors that can occur with L1 primitives,
/// such as addresses, pubkeys, and scripts.
#[derive(Debug, Clone, Error)]
pub enum ParseError {
    /// The provided pubkey is invalid.
    #[cfg(feature = "fullbtc")]
    #[error("supplied pubkey is invalid")]
    InvalidPubkey(#[from] secp256k1::Error),

    /// The provided 32-byte buffer is not a valid point on the curve.
    #[cfg(feature = "fullbtc")]
    #[error("not a valid point on the curve: {0}")]
    InvalidPoint(Buf32),

    /// Converting from an unsupported [`Address`](bitcoin::Address) type for a [`Buf32`].
    #[cfg(feature = "fullbtc")]
    #[error("only taproot addresses are supported but found {0:?}")]
    UnsupportedAddress(Option<AddressType>),
}
