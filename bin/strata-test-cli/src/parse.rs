use bdk_wallet::bitcoin::{Network, PublicKey, Transaction, XOnlyPublicKey};
use strata_asm_txs_bridge_v1::test_utils;
use strata_crypto::EvenSecretKey;
use strata_primitives::{
    buf::Buf32,
    l1::{BitcoinAddress, BitcoinAmount, DepositRequestInfo},
};

use crate::{constants::MAGIC_BYTES, error::Error};

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
    test_utils::parse_operator_keys(operator_keys).map_err(Into::into)
}

pub(crate) fn parse_drt(
    tx: &Transaction,
    address: BitcoinAddress,
    operators_pubkey: XOnlyPublicKey,
) -> Result<DepositRequestInfo, Error> {
    test_utils::parse_drt(
        tx,
        address,
        operators_pubkey,
        MAGIC_BYTES,
        BitcoinAmount::ZERO, // This parameter is unused in the test_utils implementation
    )
    .map_err(Into::into)
}

/// Generates a taproot address from operator public keys
pub(crate) fn generate_taproot_address(
    operator_wallet_pks: &[Buf32],
    network: Network,
) -> Result<(BitcoinAddress, XOnlyPublicKey), Error> {
    test_utils::generate_taproot_address(operator_wallet_pks, network).map_err(Into::into)
}

/// Parses an [`XOnlyPublicKey`] from a hex string.
pub(crate) fn parse_xonly_pk(x_only_pk: &str) -> Result<XOnlyPublicKey, Error> {
    test_utils::parse_xonly_pk(x_only_pk).map_err(Into::into)
}

/// Parses a [`PublicKey`] from a hex string.
pub(crate) fn parse_pk(pk: &str) -> Result<PublicKey, Error> {
    test_utils::parse_pk(pk).map_err(Into::into)
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
