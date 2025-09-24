//! Withdrawal fulfillment transaction functionality
//!
//! Handles the creation of withdrawal fulfillment transactions that allow operators
//! to fulfill withdrawal requests by sending Bitcoin to users.

use std::str::FromStr;

use bdk_wallet::{
    bitcoin::{
        consensus::serialize, script::PushBytesBuf, Amount, FeeRate, ScriptBuf, Transaction, Txid,
    },
    TxOrdering,
};
use strata_primitives::bitcoin_bosd::Descriptor;

use super::types::WithdrawalMetadata;
use crate::{
    constants::MAGIC_BYTES,
    error::Error,
    taproot::{new_bitcoind_client, sync_wallet, taproot_wallet},
};

/// Creates a withdrawal fulfillment transaction (CLI wrapper)
///
/// # Arguments
/// * `recipient_bosd` - bosd specifying which address to send to
/// * `amount` - Amount to send in satoshis
/// * `operator_idx` - Operator index
/// * `deposit_idx` - Deposit index
/// * `deposit_txid` - Deposit transaction ID as hex string
/// * `bitcoind_url` - Bitcoind url
/// * `bitcoind_user` - credentials
/// * `bitcoind_password` - credentials
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_withdrawal_fulfillment_cli(
    recipient_bosd: String,
    amount: u64,
    operator_idx: u32,
    deposit_idx: u32,
    deposit_txid: String,
    bitcoind_url: String,
    bitcoind_user: String,
    bitcoind_password: String,
) -> Result<Vec<u8>, Error> {
    let recipient_script = recipient_bosd
        .parse::<Descriptor>()
        .map_err(|_| Error::TxBuilder("Not a valid bosd".to_string()))?
        .to_script();

    let tx = create_withdrawal_fulfillment_inner(
        recipient_script,
        amount,
        operator_idx,
        deposit_idx,
        deposit_txid,
        &bitcoind_url,
        &bitcoind_user,
        &bitcoind_password,
    )?;

    Ok(serialize(&tx))
}

/// Internal implementation of withdrawal fulfillment creation
#[allow(clippy::too_many_arguments)]
fn create_withdrawal_fulfillment_inner(
    recipient_script: ScriptBuf,
    amount: u64,
    operator_idx: u32,
    deposit_idx: u32,
    deposit_txid: String,
    bitcoind_url: &str,
    bitcoind_user: &str,
    bitcoind_password: &str,
) -> Result<Transaction, Error> {
    // Parse inputs
    let amount = Amount::from_sat(amount);
    let deposit_txid = parse_deposit_txid(&deposit_txid)?;

    // Create withdrawal metadata
    let metadata = WithdrawalMetadata::new(*MAGIC_BYTES, operator_idx, deposit_idx, deposit_txid);

    // Create withdrawal fulfillment transaction
    create_withdrawal_transaction(
        metadata,
        recipient_script,
        amount,
        bitcoind_url,
        bitcoind_user,
        bitcoind_password,
    )
}

/// Creates the raw withdrawal transaction
fn create_withdrawal_transaction(
    metadata: WithdrawalMetadata,
    recipient_script: ScriptBuf,
    amount: Amount,
    bitcoind_url: &str,
    bitcoind_user: &str,
    bitcoind_password: &str,
) -> Result<Transaction, Error> {
    let mut wallet = taproot_wallet()?;
    let client = new_bitcoind_client(
        bitcoind_url,
        None,
        Some(bitcoind_user),
        Some(bitcoind_password),
    )?;

    sync_wallet(&mut wallet, &client)?;

    let fee_rate = FeeRate::from_sat_per_vb_unchecked(2);

    let mut psbt = {
        let mut builder = wallet.build_tx();

        builder.ordering(TxOrdering::Untouched);
        builder.add_recipient(recipient_script, amount);
        builder.add_data(&PushBytesBuf::from(&metadata.op_return_data()));

        builder.fee_rate(fee_rate);
        builder
            .finish()
            .map_err(|e| Error::TxBuilder(format!("Invalid PSBT: {e}")))?
    };

    wallet
        .sign(&mut psbt, Default::default())
        .map_err(|e| Error::TxBuilder(format!("Signing failed: {e}")))?;

    let tx = psbt
        .extract_tx()
        .map_err(|e| Error::TxBuilder(format!("Transaction extraction failed: {e}")))?;

    Ok(tx)
}

/// Parses deposit transaction ID from hex string
fn parse_deposit_txid(txid_hex: &str) -> Result<Txid, Error> {
    Txid::from_str(txid_hex)
        .map_err(|_| Error::TxBuilder("Invalid deposit transaction ID".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_deposit_txid_valid() {
        let txid = "ae86b8c8912594427bf148eb7660a86378f2fb4ac9c8d2ea7d3cb7f3fcfd7c1c";
        let parsed = parse_deposit_txid(txid);
        assert!(parsed.is_ok());

        let expected = Txid::from_str(txid).unwrap();
        assert_eq!(parsed.unwrap(), expected);
    }

    #[test]
    fn parse_deposit_txid_rejects_invalid_hex() {
        let bad = "not_a_txid";
        let err = parse_deposit_txid(bad).unwrap_err();
        match err {
            Error::TxBuilder(msg) => assert_eq!(msg, "Invalid deposit transaction ID"),
            _ => panic!("expected Error::TxBuilder"),
        }
    }

    #[test]
    fn create_withdrawal_fulfillment_inner_rejects_invalid_txid() {
        let result = create_withdrawal_fulfillment_inner(
            ScriptBuf::new(),
            1000,
            1,
            1,
            "bad_txid".to_string(),
            "http://127.0.0.1:18443",
            "user",
            "pass",
        );
        assert!(result.is_err());
    }
}
