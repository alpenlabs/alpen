use bdk_wallet::bitcoin::{
    bip32::Xpriv, taproot::TaprootBuilder, Address, Network, PublicKey, Transaction, XOnlyPublicKey,
};
use secp256k1::SECP256K1;
use strata_crypto::{multisig::aggregate_schnorr_keys, EvenSecretKey};
use strata_params::DepositTxParams;
use strata_primitives::{
    buf::Buf32,
    constants::{EE_ADDRESS_LEN, STRATA_OP_WALLET_DERIVATION_PATH},
    l1::{BitcoinAddress, BitcoinAmount, BitcoinXOnlyPublicKey, DepositRequestInfo},
};

use crate::{
    bridge::deposit_request::extract_deposit_request_info,
    constants::{BRIDGE_OUT_AMOUNT, MAGIC_BYTES},
    error::Error,
};

/// Parses operator EvenSecretKey from extended private key bytes
///
/// Takes raw bytes representing extended private keys and derives
/// operator keys using the standard key derivation path.
///
/// # Arguments
/// * `operator_keys` - Slice of 78-byte extended private key arrays
///
/// # Returns
/// * `Result<Vec<EvenSecretKey>, Error>` - Vector of derived even secret keys
pub(crate) fn parse_operator_keys(operator_keys: &[[u8; 78]]) -> Result<Vec<EvenSecretKey>, Error> {
    Ok(operator_keys
        .iter()
        .map(|bytes| {
            let xpriv = Xpriv::decode(bytes).expect("valid Xpriv bytes");

            let derived_xpriv = xpriv
                .derive_priv(SECP256K1, &STRATA_OP_WALLET_DERIVATION_PATH)
                .expect("good child key");

            EvenSecretKey::from(derived_xpriv.private_key)
        })
        .collect())
}

/// Parses a deposit request transaction (DRT) and extracts relevant information
///
/// Validates the transaction against expected parameters and extracts
/// deposit request information for further processing.
///
/// # Arguments
/// * `tx` - The Bitcoin transaction to parse
/// * `address` - Expected Bitcoin address for validation
/// * `operators_pubkey` - Aggregated public key of operators
///
/// # Returns
/// * `Result<DepositRequestInfo, Error>` - Parsed deposit request information
pub(crate) fn parse_drt(
    tx: &Transaction,
    address: BitcoinAddress,
    operators_pubkey: XOnlyPublicKey,
) -> Result<DepositRequestInfo, Error> {
    let config = DepositTxParams {
        magic_bytes: *MAGIC_BYTES,
        max_address_length: EE_ADDRESS_LEN,
        deposit_amount: BitcoinAmount::from(BRIDGE_OUT_AMOUNT),
        address,
        operators_pubkey: BitcoinXOnlyPublicKey::new(operators_pubkey.serialize().into())
            .expect("good XOnlyPublicKey"),
    };

    extract_deposit_request_info(tx, &config).ok_or(Error::TxParser("Bad DRT".to_string()))
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
