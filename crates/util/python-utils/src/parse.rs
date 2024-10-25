use bdk_wallet::bitcoin::{PublicKey, XOnlyPublicKey};
use reth_primitives::Address as RethAddress;

use crate::error::Error;

/// Parses a Execution Layer address.
pub(crate) fn parse_el_address(el_address: &str) -> Result<RethAddress, Error> {
    let el_address = el_address
        .parse::<RethAddress>()
        .map_err(|_| Error::ElAddress)?;
    Ok(el_address)
}

/// Parse an [`XOnlyPublicKey`] from a hex string.
pub(crate) fn parse_xonly_pk(x_only_pk: &str) -> Result<XOnlyPublicKey, Error> {
    x_only_pk
        .parse::<XOnlyPublicKey>()
        .map_err(|_| Error::XOnlyPublicKey)
}

/// Parse a [`PublicKey`] from a hex string.
pub(crate) fn parse_pk(pk: &str) -> Result<PublicKey, Error> {
    pk.parse::<PublicKey>().map_err(|_| Error::PublicKey)
}

#[cfg(test)]
mod tests {

    #[test]
    fn parse_el_address() {
        let el_address = "deadf001900dca3ebeefdeadf001900dca3ebeef";
        assert!(super::parse_el_address(el_address).is_ok());
        let el_address = "0xdeadf001900dca3ebeefdeadf001900dca3ebeef";
        assert!(super::parse_el_address(el_address).is_ok());
    }

    #[test]
    fn parse_xonly_pk() {
        let x_only_pk = "14ced579c6a92533fa68ccc16da93b41073993cfc6cc982320645d8e9a63ee65";
        assert!(super::parse_xonly_pk(x_only_pk).is_ok());
    }

    #[test]
    fn parse_pk() {
        let pk = "028b71ab391bc0a0f5fd8d136458e8a5bd1e035e27b8cef77b12d057b4767c31c8";
        assert!(super::parse_pk(pk).is_ok());
    }
}
