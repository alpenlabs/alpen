use bdk_wallet::bitcoin::{OutPoint, PublicKey, Transaction, Txid, XOnlyPublicKey};
use strata_l1tx::{
    deposit::deposit_request::extract_deposit_request_info, utils::generate_taproot_address,
};
use strata_primitives::{
    buf::Buf32,
    constants::EE_ADDRESS_LEN,
    l1::{DepositRequestInfo, XOnlyPk},
    params::DepositTxParams,
};

use crate::{
    constants::{BRIDGE_OUT_AMOUNT, MAGIC_BYTES, NETWORK},
    error::Error,
};

#[allow(unused)]
pub(crate) fn parse_drt(
    tx: &Transaction,
    op_wallet_pks: &[Buf32],
) -> Result<DepositRequestInfo, Error> {
    let (address, operators_pubkey) = generate_taproot_address(op_wallet_pks, NETWORK)
        .map_err(|e| Error::TxParser(e.to_string()))?;

    let config = DepositTxParams {
        magic_bytes: *MAGIC_BYTES,
        address_length: EE_ADDRESS_LEN,
        deposit_amount: BRIDGE_OUT_AMOUNT.to_sat(),
        address,
        operators_pubkey: XOnlyPk::new(operators_pubkey.serialize().into())
            .expect("good XOnlyPublicKey"),
    };

    extract_deposit_request_info(tx, &config).ok_or(Error::TxParser("Bad DRT".to_string()))
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
}
