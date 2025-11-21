use bdk_wallet::bitcoin::{bip32::Xpriv, Network, PublicKey, XOnlyPublicKey};
use secp256k1::SECP256K1;
use strata_crypto::EvenSecretKey;
use strata_l1tx::utils::generate_taproot_address as generate_taproot_address_impl;
use strata_primitives::{
    buf::Buf32, constants::STRATA_OP_WALLET_DERIVATION_PATH, l1::BitcoinAddress,
};

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
pub(crate) fn generate_taproot_address(
    operator_wallet_pks: &[Buf32],
    network: Network,
) -> Result<(BitcoinAddress, XOnlyPublicKey), Error> {
    generate_taproot_address_impl(operator_wallet_pks, network)
        .map_err(|e| Error::TxBuilder(e.to_string()))
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
