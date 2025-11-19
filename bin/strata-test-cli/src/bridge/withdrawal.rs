//! Withdrawal fulfillment transaction functionality using asm test_utils
//!
//! Handles the creation of withdrawal fulfillment transactions using the asm test_utils
//! for transaction building while keeping the wallet/UTXO management in the CLI.

use std::str::FromStr;

use bdk_wallet::{
    bitcoin::{consensus::serialize, script::PushBytesBuf, Amount, FeeRate, ScriptBuf, Txid},
    TxOrdering,
};
use strata_asm_txs_bridge_v1::test_utils::WithdrawalMetadata;
use strata_primitives::bitcoin_bosd::Descriptor;

use super::types::BitcoinDConfig;
use crate::{
    constants::MAGIC_BYTES, error::Error, taproot::{new_bitcoind_client, sync_wallet, taproot_wallet},
};

/// Creates a withdrawal fulfillment transaction (CLI wrapper)
///
/// This function handles wallet operations (UTXO selection, signing) while using
/// the test_utils for transaction structure creation.
///
/// # Arguments
/// * `recipient_bosd` - bosd specifying which address to send to
/// * `amount` - Amount to send in satoshis
/// * `operator_idx` - Operator index
/// * `deposit_idx` - Deposit index
/// * `deposit_txid` - Deposit transaction ID as hex string
/// * `bitcoind_config` - Bitcoind config
pub(crate) fn create_withdrawal_fulfillment_cli(
    recipient_bosd: String,
    amount: u64,
    operator_idx: u32,
    deposit_idx: u32,
    deposit_txid: String,
    bitcoind_config: BitcoinDConfig,
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
        bitcoind_config,
    )?;

    Ok(serialize(&tx))
}

/// Internal implementation of withdrawal fulfillment creation
fn create_withdrawal_fulfillment_inner(
    recipient_script: ScriptBuf,
    amount: u64,
    operator_idx: u32,
    deposit_idx: u32,
    deposit_txid: String,
    bitcoind_config: BitcoinDConfig,
) -> Result<bdk_wallet::bitcoin::Transaction, Error> {
    // Parse inputs
    let amount = Amount::from_sat(amount);
    let deposit_txid = parse_deposit_txid(&deposit_txid)?;

    // Create withdrawal metadata
    let metadata = WithdrawalMetadata::new(*MAGIC_BYTES, operator_idx, deposit_idx, deposit_txid);

    // Use wallet to select and fund inputs (CLI responsibility)
    let mut wallet = taproot_wallet()?;
    let client = new_bitcoind_client(
        &bitcoind_config.bitcoind_url,
        None,
        Some(&bitcoind_config.bitcoind_user),
        Some(&bitcoind_config.bitcoind_password),
    )?;

    sync_wallet(&mut wallet, &client)?;

    let fee_rate = FeeRate::from_sat_per_vb_unchecked(2);

    // Build PSBT using wallet for funding
    let mut psbt = {
        let mut builder = wallet.build_tx();

        builder.ordering(TxOrdering::Untouched);
        let op_return_data = metadata
            .op_return_data()
            .map_err(|e| Error::TxBuilder(format!("Failed to create OP_RETURN data: {e}")))?;
        let push_bytes = PushBytesBuf::try_from(op_return_data)
            .map_err(|_| Error::TxBuilder("OP_RETURN data too large".to_string()))?;
        builder.add_data(&push_bytes);
        builder.add_recipient(recipient_script.clone(), amount);

        builder.fee_rate(fee_rate);
        builder
            .finish()
            .map_err(|e| Error::TxBuilder(format!("Invalid PSBT: {e}")))?
    };

    // Sign the PSBT
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
}
