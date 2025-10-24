use thiserror::Error;

/// Error types for test CLI operations
#[derive(Debug, Clone, Error)]
pub(crate) enum Error {
    #[error("Could not create wallet")]
    Wallet,

    #[error("Invalid X-only public key")]
    XOnlyPublicKey,

    #[error("Invalid public key")]
    PublicKey,

    #[error("Not a P2TR address")]
    NotTaprootAddress,

    #[error("Could not create RPC client")]
    RpcClient,

    #[error("Invalid BitcoinD response")]
    BitcoinD,

    #[error("Transaction builder error: {0}")]
    TxBuilder(String),

    #[error("MuSig error: {0}")]
    Musig(String),

    #[error("Transaction parser error: {0}")]
    TxParser(String),

    #[error("Invalid hex string: {0}")]
    InvalidHex(String),

    #[error("Invalid JSON: {0}")]
    InvalidJson(String),

    #[error("Invalid operator key length: expected 78 bytes, got {0}")]
    InvalidKeyLength(usize),

    #[error("Invalid DRT transaction hex: {0}")]
    InvalidDrtHex(String),

    #[error("Invalid operator keys JSON: {0}")]
    InvalidOperatorKeysJson(String),

    #[error("Invalid pubkeys JSON: {0}")]
    InvalidPubkeysJson(String),
}

impl From<hex::FromHexError> for Error {
    fn from(e: hex::FromHexError) -> Self {
        Error::InvalidHex(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::InvalidJson(e.to_string())
    }
}

impl From<secp256k1::Error> for Error {
    fn from(_: secp256k1::Error) -> Self {
        Error::PublicKey
    }
}
