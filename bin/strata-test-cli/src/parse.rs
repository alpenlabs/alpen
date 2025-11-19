use bdk_wallet::bitcoin::{
    bip32::Xpriv,
    taproot::TaprootBuilder,
    Address, Network, PublicKey, Transaction, XOnlyPublicKey,
};
use secp256k1::SECP256K1;
use strata_asm_common::TxInputRef;
use strata_asm_txs_bridge_v1::deposit_request;
use strata_crypto::{EvenSecretKey, multisig::aggregate_schnorr_keys};
use strata_l1_txfmt::ParseConfig;
use strata_primitives::{
    buf::Buf32,
    constants::STRATA_OP_WALLET_DERIVATION_PATH,
    l1::{BitcoinAddress, DepositRequestInfo},
};

use crate::{constants::MAGIC_BYTES, error::Error};

/// Parses operator extended private keys and derives the wallet keys
pub(crate) fn parse_operator_keys(operator_keys: &[[u8; 78]]) -> Result<Vec<EvenSecretKey>, Error> {
    operator_keys
        .iter()
        .map(|bytes| {
            let xpriv = Xpriv::decode(bytes).map_err(|_| Error::InvalidXpriv)?;

            let derived_xpriv = xpriv
                .derive_priv(SECP256K1, &STRATA_OP_WALLET_DERIVATION_PATH)
                .map_err(|_| Error::InvalidXpriv)?;

            Ok(EvenSecretKey::from(derived_xpriv.private_key))
        })
        .collect::<Result<Vec<_>, Error>>()
}

pub(crate) fn parse_drt(
    tx: &Transaction,
    _address: BitcoinAddress,
    _operators_pubkey: XOnlyPublicKey,
) -> Result<DepositRequestInfo, Error> {
    let parse_config = ParseConfig::new(*MAGIC_BYTES);

    // Find the OP_RETURN output index
    let op_return_idx = tx
        .output
        .iter()
        .position(|output| output.script_pubkey.is_op_return())
        .ok_or_else(|| Error::TxParser("No OP_RETURN output found in DRT".to_string()))?;

    // Create a temporary transaction with the OP_RETURN at index 0 for parsing
    let mut temp_tx = tx.clone();
    if op_return_idx != 0 {
        temp_tx.output.swap(0, op_return_idx);
    }

    let tag_data = parse_config
        .try_parse_tx(&temp_tx)
        .map_err(|e| Error::TxParser(format!("Failed to parse SPS-50 transaction: {}", e)))?;

    // Use original transaction for creating TxInputRef
    let tx_input = TxInputRef::new(tx, tag_data);
    deposit_request::parse_drt(&tx_input).map_err(|e| Error::TxParser(format!("Bad DRT: {}", e)))
}

/// Generates a taproot address from operator public keys
pub(crate) fn generate_taproot_address(
    operator_wallet_pks: &[Buf32],
    network: Network,
) -> Result<(BitcoinAddress, XOnlyPublicKey), Error> {
    let x_only_pub_key = aggregate_schnorr_keys(operator_wallet_pks.iter())
        .map_err(|e| Error::TxBuilder(format!("aggregate schnorr keys: {}", e)))?;

    let taproot_builder = TaprootBuilder::new();
    let spend_info = taproot_builder
        .finalize(SECP256K1, x_only_pub_key)
        .map_err(|_| Error::TxBuilder("taproot finalization".to_string()))?;
    let merkle_root = spend_info.merkle_root();

    let addr = Address::p2tr(SECP256K1, x_only_pub_key, merkle_root, network);
    let addr = BitcoinAddress::parse(&addr.to_string(), network)
        .map_err(|e| Error::TxBuilder(format!("parse bitcoin address: {}", e)))?;

    Ok((addr, x_only_pub_key))
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
}
