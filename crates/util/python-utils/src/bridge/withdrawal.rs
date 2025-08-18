//! Withdrawal fulfillment transaction functionality
//!
//! Handles the creation of withdrawal fulfillment transactions that allow operators
//! to fulfill withdrawal requests by sending Bitcoin to users.

use bdk_wallet::{bitcoin::{consensus::serialize, Amount, FeeRate, ScriptBuf, Transaction, Txid}, TxOrdering};
use pyo3::prelude::*;
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::{error::Error, taproot::{new_bitcoind_client, sync_wallet, taproot_wallet}};
use super::types::WithdrawalMetadata;
use std::str::FromStr;

/// Creates a withdrawal fulfillment transaction
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
#[pyfunction]
pub(crate) fn create_withdrawal_fulfillment(
    recipient_bosd: String,
    amount: u64,
    operator_idx: u32,
    deposit_idx: u32,
    deposit_txid: String,
    bitcoind_url: &str,
    bitcoind_user: &str,
    bitcoind_password: &str,
) -> PyResult<Vec<u8>> {

    let recipient_script = recipient_bosd
        .parse::<Descriptor>()
        .expect("Not a valid bosd")
        .to_script();


    let tx = create_withdrawal_fulfillment_inner(
        recipient_script,
        amount,
        operator_idx,
        deposit_idx,
        deposit_txid,
        bitcoind_url,
        bitcoind_user,
        bitcoind_password
    )?;

    let serialized_tx = serialize(&tx);
    Ok(serialized_tx)
}

#[allow(clippy::too_many_arguments)]
/// Internal implementation of withdrawal fulfillment creation
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
    let tag = *b"ALPN"; // Alpen rollup tag
    let metadata = WithdrawalMetadata::new(
        tag,
        operator_idx,
        deposit_idx,
        deposit_txid,
    );

    // Create withdrawal fulfillment transaction
    let withdrawal_fulfillment =
        create_withdrawal_transaction(
            metadata,
            recipient_script,
            amount,
            bitcoind_url,
            bitcoind_user,
            bitcoind_password,
            )
        .unwrap();

    Ok(withdrawal_fulfillment)
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

    // Create outputs
    let fee_rate = FeeRate::from_sat_per_vb_unchecked(2);

    let mut psbt = {
        let mut builder = wallet.build_tx();

        builder.ordering(TxOrdering::Untouched);
        builder.add_recipient(recipient_script,amount);
        builder.add_data(&metadata.op_return_script());

        builder.fee_rate(fee_rate);
        builder.finish().expect("withdrawal fulfillment: invalid psbt")
    };

    wallet.sign(&mut psbt, Default::default()).unwrap();

    let tx = psbt.extract_tx().expect("withdrawal fulfillment: invalid transaction");

    Ok(tx)
}

/// Parses deposit transaction ID from hex string
fn parse_deposit_txid(txid_hex: &str) -> Result<Txid, Error> {
    Txid::from_str(txid_hex)
        .map_err(|_| Error::BridgeBuilder("Invalid deposit transaction ID".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_deposit_txid() {
        let txid = "ae86b8c8912594427bf148eb7660a86378f2fb4ac9c8d2ea7d3cb7f3fcfd7c1c";
        assert!(parse_deposit_txid(txid).is_ok());
    }
}
