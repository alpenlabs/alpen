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
}

impl From<secp256k1::Error> for Error {
    fn from(_: secp256k1::Error) -> Self {
        Error::PublicKey
    }
}
