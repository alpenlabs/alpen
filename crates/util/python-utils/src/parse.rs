use bdk_wallet::bitcoin::{Address, OutPoint, PublicKey, Txid, XOnlyPublicKey};
use revm_primitives::alloy_primitives::Address as RethAddress;

use crate::error::Error;

/// Parses a Execution Layer address.
pub(crate) fn parse_el_address(el_address: &str) -> Result<RethAddress, Error> {
    let el_address = el_address
        .parse::<RethAddress>()
        .map_err(|_| Error::ElAddress)?;
    Ok(el_address)
}

/// Parses an [`XOnlyPublicKey`] from a hex string.
pub(crate) fn parse_xonly_pk(x_only_pk: &str) -> Result<XOnlyPublicKey, Error> {
    x_only_pk
        .parse::<XOnlyPublicKey>()
        .map_err(|_| Error::XOnlyPublicKey)
}

/// Parses a [`PublicKey`] from a hex string.
pub(crate) fn parse_pk(pk: &str) -> Result<PublicKey, Error> {
    pk.parse::<PublicKey>().map_err(|_| Error::PublicKey)
}

/// Parses an [`Address`] from a string.
pub(super) fn parse_address(address: &str) -> Result<Address, Error> {
    Ok(address
        .parse::<Address<_>>()
        .map_err(|_| Error::BitcoinAddress)?
        .assume_checked())
}

/// Parses an [`OutPoint`] from a string.
#[allow(dead_code)] // This might be useful in the future
pub(crate) fn parse_outpoint(outpoint: &str) -> Result<OutPoint, Error> {
    let parts: Vec<&str> = outpoint.split(':').collect();
    if parts.len() != 2 {
        return Err(Error::OutPoint);
    }
    let txid = parts[0].parse::<Txid>().map_err(|_| Error::OutPoint)?;
    let vout = parts[1].parse::<u32>().map_err(|_| Error::OutPoint)?;
    Ok(OutPoint { txid, vout })
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

    #[test]
    fn parse_outpoint() {
        let outpoint = "ae86b8c8912594427bf148eb7660a86378f2fb4ac9c8d2ea7d3cb7f3fcfd7c1c:0";
        assert!(super::parse_outpoint(outpoint).is_ok());
        let outpoint_without_vout =
            "ae86b8c8912594427bf148eb7660a86378f2fb4ac9c8d2ea7d3cb7f3fcfd7c1c";
        assert!(super::parse_outpoint(outpoint_without_vout).is_err());
        let outpoint_with_vout_out_of_bonds = {
            let vout = u32::MAX as u64 + 1;
            format!("ae86b8c8912594427bf148eb7660a86378f2fb4ac9c8d2ea7d3cb7f3fcfd7c1c:{vout}")
        };
        assert!(super::parse_outpoint(&outpoint_with_vout_out_of_bonds).is_err());
    }

    #[test]
    fn parse_address() {
        let address = "bcrt1phcnl4zcl2fu047pv4wx6y058v8u0n02at6lthvm7pcf2wrvjm5tqatn90k";
        assert!(super::parse_address(address).is_ok());
    }
}
