use bdk_wallet::bitcoin::{
    bip32::Xpriv, secp256k1::SECP256K1, taproot::TaprootBuilder, Address, Network, PublicKey,
    XOnlyPublicKey,
};
use strata_crypto::{
    aggregate_schnorr_keys, keys::constants::STRATA_OP_WALLET_DERIVATION_PATH, EvenSecretKey,
};
use strata_primitives::{buf::Buf32, l1::BitcoinAddress};

use crate::error::Error;

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

/// Generates a taproot address from operator public keys
///
/// Creates a taproot address by aggregating the operator wallet public keys,
/// building a taproot spending tree, and deriving the final address.
///
/// # Arguments
/// * `operator_wallet_pks` - Slice of operator wallet public keys to aggregate
/// * `network` - Bitcoin network (mainnet, testnet, regtest, etc.)
///
/// # Returns
/// * `Result<(BitcoinAddress, XOnlyPublicKey), Error>` - Taproot address and internal pubkey
pub(crate) fn generate_taproot_address(
    operator_wallet_pks: &[Buf32],
    network: Network,
) -> Result<(BitcoinAddress, XOnlyPublicKey), Error> {
    // Aggregate the operator public keys into a single x-only pubkey
    let x_only_pub_key = aggregate_schnorr_keys(operator_wallet_pks.iter())
        .map_err(|e| Error::TxBuilder(format!("Failed to aggregate keys: {}", e)))?;

    // Build the taproot spending tree (empty tree in this case)
    let taproot_builder = TaprootBuilder::new();
    let spend_info = taproot_builder
        .finalize(SECP256K1, x_only_pub_key)
        .map_err(|_| Error::TxBuilder("Taproot finalization failed".to_string()))?;
    let merkle_root = spend_info.merkle_root();

    // Create the P2TR address
    let addr = Address::p2tr(SECP256K1, x_only_pub_key, merkle_root, network);
    let addr = BitcoinAddress::parse(&addr.to_string(), network)
        .map_err(|e| Error::TxBuilder(format!("Failed to parse address: {}", e)))?;

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
