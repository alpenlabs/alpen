use pyo3::{exceptions::PyValueError, prelude::*};

/// Error types for the functional tests.
#[derive(Debug, Clone)]
pub(crate) enum Error {
    /// Could not create a wallet.
    Wallet,

    /// Invalid XOnlyPublicKey.
    XOnlyPublicKey,

    /// Invalid PublicKey
    PublicKey,

    /// Invalid Outpoint.
    OutPoint,

    /// Not a Taproot address.
    NotTaprootAddress,

    /// Invalid Bitcoin address.
    BitcoinAddress,

    /// `OP_RETURN` bigger than 80 bytes.
    OpReturnTooLong,

    /// Could not create a BitcoinD RPC client.
    RpcClient,

    /// Error with BitcoinD response.
    BitcoinD,

    /// Bridge transaction builder error.
    #[allow(dead_code)]
    TxBuilder(String),

    /// Musig2 error
    Musig(String),

    /// Error related to Transaction Parsing
    TxParser(String),
}

/// Converts an `Error` into a `PyErr` to be raised in Python.
impl From<Error> for PyErr {
    fn from(err: Error) -> PyErr {
        let msg = match err {
            Error::Wallet => "Could not create wallet",
            Error::XOnlyPublicKey => "Invalid X-only public key",
            Error::PublicKey => "Invalid public key",
            Error::OutPoint => "Invalid outpoint",
            Error::NotTaprootAddress => "Not a P2TR address",
            Error::BitcoinAddress => "Not a valid bitcoin address",
            Error::OpReturnTooLong => "OP_RETURN bigger than 80 bytes",
            Error::RpcClient => "Could not create RPC client",
            Error::BitcoinD => "Invalid BitcoinD response",
            Error::TxBuilder(ref msg) => msg,
            Error::Musig(ref msg) => msg,
            Error::TxParser(ref msg) => msg,
        };
        PyErr::new::<PyValueError, _>(msg.to_owned())
    }
}
