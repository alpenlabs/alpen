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

    #[error("Invalid extended private key")]
    InvalidXpriv,

    #[error("Not a P2TR address")]
    NotTaprootAddress,

    #[error("Could not create RPC client")]
    RpcClient,

    #[error("Invalid BitcoinD response")]
    BitcoinD,

    #[error("Transaction builder error: {0}")]
    TxBuilder(String),

    #[error("Transaction parser error: {0}")]
    TxParser(String),

    #[error("Invalid hex string: {0}")]
    InvalidHex(String),

    #[error("Invalid JSON: {0}")]
    InvalidJson(String),
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
