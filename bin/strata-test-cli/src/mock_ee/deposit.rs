//! Mock deposit injection via debug subprotocol.
//!
//! Creates a Bitcoin transaction that injects a [`DepositIntentLogData`] into the ASM
//! via the debug subprotocol's MockAsmLog mechanism (subprotocol ID 255, tx_type 1).

use bdk_wallet::bitcoin::{ScriptBuf, consensus::serialize, Amount, FeeRate, Transaction};
use strata_asm_manifest_types::DepositIntentLogData;
use strata_identifiers::{AccountSerial, SubjectId};
use strata_l1_txfmt::{ParseConfig, TagDataRef};

use crate::{
    bridge::types::BitcoinDConfig,
    constants::MAGIC_BYTES,
    error::Error,
    taproot::{new_bitcoind_client, sync_wallet, taproot_wallet},
};

/// Debug subprotocol ID (u8::MAX = 255).
const DEBUG_SUBPROTOCOL_ID: u8 = u8::MAX;

/// MockAsmLog transaction type within the debug subprotocol.
const MOCK_ASM_LOG_TX_TYPE: u8 = 1;

/// Creates a mock deposit Bitcoin transaction and returns the serialized transaction bytes.
///
/// The transaction contains an OP_RETURN output with an SPS-50 tag encoding:
/// - Subprotocol ID: 255 (debug)
/// - Tx type: 1 (MockAsmLog)
/// - Aux data: encoded `DepositIntentLogData` as an `AsmLogEntry`
pub(crate) fn create_mock_deposit_tx(
    account_serial: u32,
    amount: u64,
    bitcoind_config: BitcoinDConfig,
) -> Result<Vec<u8>, Error> {
    // Build the deposit intent log data
    let deposit_log_data = DepositIntentLogData::new(
        AccountSerial::from(account_serial),
        SubjectId::from([0u8; 32]),
        amount,
    );

    // Encode as AsmLogEntry via the AsmLog trait
    let log_entry = strata_asm_manifest_types::AsmLogEntry::from_log(&deposit_log_data)
        .map_err(|e| Error::TxBuilder(format!("failed to encode deposit log: {e}")))?;
    let raw_bytes = log_entry.into_bytes();

    // Build SPS-50 tag: debug subprotocol (255), MockAsmLog tx_type (1), aux = raw log bytes
    let tag = TagDataRef::new(DEBUG_SUBPROTOCOL_ID, MOCK_ASM_LOG_TX_TYPE, &raw_bytes)
        .map_err(|e| Error::TxBuilder(format!("failed to build tag data: {e}")))?;

    // Encode into OP_RETURN script
    let op_return_script = ParseConfig::new(MAGIC_BYTES)
        .encode_script_buf(&tag)
        .map_err(|e| Error::TxBuilder(format!("failed to encode OP_RETURN: {e}")))?;

    // Build and sign the Bitcoin transaction using BDK wallet
    let tx = build_and_sign_tx(op_return_script, bitcoind_config)?;

    Ok(serialize(&tx))
}

/// Builds the SPS-50 OP_RETURN script for a mock deposit.
///
/// Exposed for unit testing. Encodes `DepositIntentLogData` into
/// a debug subprotocol tagged script.
#[cfg(test)]
pub(crate) fn build_mock_deposit_op_return(
    account_serial: u32,
    amount: u64,
) -> Result<ScriptBuf, Error> {
    let deposit_log_data = DepositIntentLogData::new(
        AccountSerial::from(account_serial),
        SubjectId::from([0u8; 32]),
        amount,
    );

    let log_entry = strata_asm_manifest_types::AsmLogEntry::from_log(&deposit_log_data)
        .map_err(|e| Error::TxBuilder(format!("failed to encode deposit log: {e}")))?;
    let raw_bytes = log_entry.into_bytes();

    let tag = TagDataRef::new(DEBUG_SUBPROTOCOL_ID, MOCK_ASM_LOG_TX_TYPE, &raw_bytes)
        .map_err(|e| Error::TxBuilder(format!("failed to build tag data: {e}")))?;

    ParseConfig::new(MAGIC_BYTES)
        .encode_script_buf(&tag)
        .map_err(|e| Error::TxBuilder(format!("failed to encode OP_RETURN: {e}")))
}

/// Builds a Bitcoin transaction with the given OP_RETURN script, funds it from
/// the regtest wallet, signs, and returns the finalized transaction.
fn build_and_sign_tx(
    op_return_script: ScriptBuf,
    bitcoind_config: BitcoinDConfig,
) -> Result<Transaction, Error> {
    let mut wallet = taproot_wallet()?;
    let client = new_bitcoind_client(
        &bitcoind_config.bitcoind_url,
        None,
        Some(&bitcoind_config.bitcoind_user),
        Some(&bitcoind_config.bitcoind_password),
    )?;

    sync_wallet(&mut wallet, &client)?;

    let fee_rate = FeeRate::from_sat_per_vb_unchecked(2);

    let mut psbt = {
        let mut builder = wallet.build_tx();
        builder.add_recipient(op_return_script, Amount::ZERO);
        builder.fee_rate(fee_rate);
        builder
            .finish()
            .map_err(|e| Error::TxBuilder(format!("failed to build PSBT: {e}")))?
    };

    wallet
        .sign(&mut psbt, Default::default())
        .map_err(|e| Error::TxBuilder(format!("signing failed: {e}")))?;

    psbt.extract_tx()
        .map_err(|e| Error::TxBuilder(format!("tx extraction failed: {e}")))
}

#[cfg(test)]
mod tests {
    use strata_asm_manifest_types::DepositIntentLogData;

    use super::*;

    #[test]
    fn test_deposit_op_return_script_is_valid() {
        let script = build_mock_deposit_op_return(0x42, 100_000_000)
            .expect("should build op_return script");

        // Should start with OP_RETURN
        assert!(script.is_op_return(), "script should be OP_RETURN");

        // Should be non-trivial in size (magic bytes + tag header + encoded log)
        assert!(script.len() > 10, "script should have meaningful content");
    }

    #[test]
    fn test_deposit_log_roundtrip_through_asm_log_entry() {
        let serial = 0x42u32;
        let amount = 200_000_000u64;

        let deposit_log_data = DepositIntentLogData::new(
            AccountSerial::from(serial),
            SubjectId::from([0u8; 32]),
            amount,
        );

        // Encode to AsmLogEntry
        let log_entry = strata_asm_manifest_types::AsmLogEntry::from_log(&deposit_log_data)
            .expect("should encode");

        // Decode back
        let decoded: DepositIntentLogData = log_entry
            .try_into_log()
            .expect("should decode back to DepositIntentLogData");

        assert_eq!(decoded.dest_acct_serial(), AccountSerial::from(serial));
        assert_eq!(decoded.amt(), amount);
    }

    #[test]
    fn test_deposit_tag_data_structure() {
        let deposit_log_data = DepositIntentLogData::new(
            AccountSerial::from(1),
            SubjectId::from([0u8; 32]),
            50_000_000,
        );

        let log_entry = strata_asm_manifest_types::AsmLogEntry::from_log(&deposit_log_data)
            .expect("should encode");
        let raw_bytes = log_entry.into_bytes();

        // Verify we can construct the tag with debug subprotocol params
        let tag = TagDataRef::new(DEBUG_SUBPROTOCOL_ID, MOCK_ASM_LOG_TX_TYPE, &raw_bytes)
            .expect("should build tag");

        assert_eq!(tag.subproto_id(), DEBUG_SUBPROTOCOL_ID);
        assert_eq!(tag.tx_type(), MOCK_ASM_LOG_TX_TYPE);
        assert_eq!(tag.aux_data(), raw_bytes.as_slice());
    }
}
