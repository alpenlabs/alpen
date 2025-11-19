//! Parsing utilities for DRTs and operator keys

use bitcoin::{
    Address, Network, PublicKey, Transaction, XOnlyPublicKey,
    bip32::Xpriv,
    consensus::deserialize,
    taproot::TaprootBuilder,
};
use secp256k1::SECP256K1;
use strata_asm_common::TxInputRef;
use strata_crypto::{EvenSecretKey, multisig::aggregate_schnorr_keys};
use strata_l1_txfmt::ParseConfig;
use strata_primitives::{
    buf::Buf32,
    constants::STRATA_OP_WALLET_DERIVATION_PATH,
    l1::{BitcoinAddress, BitcoinAmount, DepositRequestInfo},
};

/// Error type for parsing operations
#[derive(Debug, Clone, thiserror::Error)]
pub enum ParsingError {
    #[error("Invalid transaction bytes: {0}")]
    InvalidTransaction(String),

    #[error("Invalid extended private key")]
    InvalidXpriv,

    #[error("Invalid X-only public key")]
    InvalidXOnlyPublicKey,

    #[error("Invalid public key")]
    InvalidPublicKey,

    #[error("Invalid deposit request transaction")]
    InvalidDRT,

    #[error("Key aggregation failed: {0}")]
    KeyAggregation(String),

    #[error("Taproot finalization failed")]
    TaprootFinalization,

    #[error("Address parsing failed: {0}")]
    AddressParsing(String),
}

pub fn parse_operator_keys(operator_keys: &[[u8; 78]]) -> Result<Vec<EvenSecretKey>, ParsingError> {
    operator_keys
        .iter()
        .map(|bytes| {
            let xpriv = Xpriv::decode(bytes).map_err(|_| ParsingError::InvalidXpriv)?;

            let derived_xpriv = xpriv
                .derive_priv(SECP256K1, &STRATA_OP_WALLET_DERIVATION_PATH)
                .map_err(|_| ParsingError::InvalidXpriv)?;

            Ok(EvenSecretKey::from(derived_xpriv.private_key))
        })
        .collect::<Result<Vec<_>, ParsingError>>()
}

pub fn generate_taproot_address(
    operator_wallet_pks: &[Buf32],
    network: Network,
) -> Result<(BitcoinAddress, XOnlyPublicKey), ParsingError> {
    let x_only_pub_key = aggregate_schnorr_keys(operator_wallet_pks.iter())
        .map_err(|e| ParsingError::KeyAggregation(format!("aggregate schnorr keys: {}", e)))?;

    let taproot_builder = TaprootBuilder::new();
    let spend_info = taproot_builder
        .finalize(SECP256K1, x_only_pub_key)
        .map_err(|_| ParsingError::TaprootFinalization)?;
    let merkle_root = spend_info.merkle_root();

    let addr = Address::p2tr(SECP256K1, x_only_pub_key, merkle_root, network);
    let addr = BitcoinAddress::parse(&addr.to_string(), network)
        .map_err(|e| ParsingError::AddressParsing(format!("parse bitcoin address: {}", e)))?;

    Ok((addr, x_only_pub_key))
}

pub fn parse_drt(
    tx: &Transaction,
    _address: BitcoinAddress,
    _operators_pubkey: XOnlyPublicKey,
    magic_bytes: &[u8; 4],
    _bridge_out_amount: BitcoinAmount,
) -> Result<DepositRequestInfo, ParsingError> {
    // Parse the transaction using SPS-50 format
    let parse_config = ParseConfig::new(*magic_bytes);
    let tag_data = parse_config
        .try_parse_tx(tx)
        .map_err(|e| ParsingError::InvalidTransaction(format!("Failed to parse SPS-50 transaction: {}", e)))?;

    // Create TxInputRef and parse DRT
    let tx_input = TxInputRef::new(tx, tag_data);
    crate::deposit_request::parse::parse_drt(&tx_input).map_err(|e| match e {
        crate::deposit_request::parse::DepositRequestParseError::InvalidTxType { .. } => {
            ParsingError::InvalidDRT
        }
        crate::deposit_request::parse::DepositRequestParseError::InvalidAuxiliaryData { .. } => {
            ParsingError::InvalidDRT
        }
        crate::deposit_request::parse::DepositRequestParseError::MissingDRTOutput => {
            ParsingError::InvalidDRT
        }
    })
}

pub fn parse_xonly_pk(x_only_pk: &str) -> Result<XOnlyPublicKey, ParsingError> {
    x_only_pk
        .parse::<XOnlyPublicKey>()
        .map_err(|_| ParsingError::InvalidXOnlyPublicKey)
}

pub fn parse_pk(pk: &str) -> Result<PublicKey, ParsingError> {
    pk.parse::<PublicKey>()
        .map_err(|_| ParsingError::InvalidPublicKey)
}

pub fn parse_transaction(tx_bytes: &[u8]) -> Result<Transaction, ParsingError> {
    deserialize::<Transaction>(tx_bytes)
        .map_err(|e| ParsingError::InvalidTransaction(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_xonly_pk() {
        let x_only_pk = "14ced579c6a92533fa68ccc16da93b41073993cfc6cc982320645d8e9a63ee65";
        assert!(parse_xonly_pk(x_only_pk).is_ok());
    }

    #[test]
    fn test_parse_pk() {
        let pk = "028b71ab391bc0a0f5fd8d136458e8a5bd1e035e27b8cef77b12d057b4767c31c8";
        assert!(parse_pk(pk).is_ok());
    }

    #[test]
    fn test_parse_invalid_xonly_pk() {
        let bad_pk = "not_a_pubkey";
        assert!(parse_xonly_pk(bad_pk).is_err());
    }
}
