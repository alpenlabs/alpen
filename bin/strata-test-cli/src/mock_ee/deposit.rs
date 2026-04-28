//! Mock deposit injection via debug subprotocol.
//!
//! Creates a Bitcoin transaction that injects a [`DepositLog`] into the ASM
//! via the debug subprotocol's MockAsmLog mechanism (subprotocol ID 255, tx_type 1).

use anyhow::Context;
use bdk_wallet::{
    bitcoin::{consensus::serialize, Amount, FeeRate, ScriptBuf, Transaction},
    TxOrdering,
};
use strata_asm_logs::DepositLog;
use strata_identifiers::{AccountSerial, SubjectIdBytes};
use strata_l1_txfmt::{ParseConfig, TagDataRef};
use strata_ol_bridge_types::DepositDescriptor;

use crate::{
    bridge::types::BitcoinDConfig,
    constants::MAGIC_BYTES,
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
/// - Aux data: encoded `DepositLog` as an `AsmLogEntry`
pub(crate) fn create_mock_deposit_tx(
    account_serial: u32,
    amount: u64,
    bitcoind_config: BitcoinDConfig,
) -> anyhow::Result<Vec<u8>> {
    // Build the deposit descriptor and encode it as bridge-v1 would
    let descriptor = DepositDescriptor::new(
        AccountSerial::from(account_serial),
        SubjectIdBytes::try_new(vec![0u8; 32]).context("failed to create subject bytes")?,
    )
    .context("failed to create deposit descriptor")?;

    let deposit_log = DepositLog::new(descriptor.encode_to_varvec(), amount);

    // Encode as AsmLogEntry via the AsmLog trait
    let log_entry = strata_asm_manifest_types::AsmLogEntry::from_log(&deposit_log)
        .context("failed to encode deposit log")?;
    let raw_bytes = log_entry.into_bytes();

    // Build SPS-50 tag: debug subprotocol (255), MockAsmLog tx_type (1), aux = raw log bytes
    let tag = TagDataRef::new(DEBUG_SUBPROTOCOL_ID, MOCK_ASM_LOG_TX_TYPE, &raw_bytes)
        .context("failed to build tag data")?;

    // Encode into OP_RETURN script
    let op_return_script = ParseConfig::new(MAGIC_BYTES)
        .encode_script_buf(&tag)
        .context("failed to encode OP_RETURN")?;

    // Build and sign the Bitcoin transaction using BDK wallet
    let tx = build_and_sign_tx(op_return_script, bitcoind_config)?;

    Ok(serialize(&tx))
}

/// Builds the SPS-50 OP_RETURN script for a mock deposit.
///
/// Exposed for unit testing. Encodes `DepositLog` into
/// a debug subprotocol tagged script.
#[cfg(test)]
pub(crate) fn build_mock_deposit_op_return(
    account_serial: u32,
    amount: u64,
) -> anyhow::Result<ScriptBuf> {
    let descriptor = DepositDescriptor::new(
        AccountSerial::from(account_serial),
        SubjectIdBytes::try_new(vec![0u8; 32]).context("failed to create subject bytes")?,
    )
    .context("failed to create deposit descriptor")?;

    let deposit_log = DepositLog::new(descriptor.encode_to_varvec(), amount);

    let log_entry = strata_asm_manifest_types::AsmLogEntry::from_log(&deposit_log)
        .context("failed to encode deposit log")?;
    let raw_bytes = log_entry.into_bytes();

    let tag = TagDataRef::new(DEBUG_SUBPROTOCOL_ID, MOCK_ASM_LOG_TX_TYPE, &raw_bytes)
        .context("failed to build tag data")?;

    ParseConfig::new(MAGIC_BYTES)
        .encode_script_buf(&tag)
        .context("failed to encode OP_RETURN")
}

/// Builds a Bitcoin transaction with the given OP_RETURN script, funds it from
/// the regtest wallet, signs, and returns the finalized transaction.
fn build_and_sign_tx(
    op_return_script: ScriptBuf,
    bitcoind_config: BitcoinDConfig,
) -> anyhow::Result<Transaction> {
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
        // Preserve insertion order so the OP_RETURN is the first output.
        // The SPS-50 parser requires the tag output at index 0.
        builder.ordering(TxOrdering::Untouched);
        builder.finish().context("failed to build PSBT")?
    };

    wallet
        .sign(&mut psbt, Default::default())
        .context("signing failed")?;

    psbt.extract_tx().context("tx extraction failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deposit_op_return_script_is_valid() {
        let script =
            build_mock_deposit_op_return(0x42, 100_000_000).expect("should build op_return script");

        // Should start with OP_RETURN
        assert!(script.is_op_return(), "script should be OP_RETURN");

        // Should be non-trivial in size (magic bytes + tag header + encoded log)
        assert!(script.len() > 10, "script should have meaningful content");
    }

    #[test]
    fn test_deposit_log_roundtrip_through_asm_log_entry() {
        let serial = 0x42u32;
        let amount = 200_000_000u64;

        let descriptor = DepositDescriptor::new(
            AccountSerial::from(serial),
            SubjectIdBytes::try_new(vec![0u8; 32]).expect("valid subject bytes"),
        )
        .expect("valid descriptor");

        let deposit_log = DepositLog::new(descriptor.encode_to_varvec(), amount);

        // Encode to AsmLogEntry
        let log_entry =
            strata_asm_manifest_types::AsmLogEntry::from_log(&deposit_log).expect("should encode");

        // Decode back
        let decoded: DepositLog = log_entry
            .try_into_log()
            .expect("should decode back to DepositLog");

        // Verify the descriptor roundtrips correctly
        let decoded_descriptor =
            DepositDescriptor::decode_from_slice(&decoded.destination).expect("valid descriptor");
        assert_eq!(
            decoded_descriptor.dest_acct_serial(),
            &AccountSerial::from(serial)
        );
        assert_eq!(decoded.amount, amount);
    }

    #[test]
    fn test_deposit_tag_data_structure() {
        let descriptor = DepositDescriptor::new(
            AccountSerial::from(1),
            SubjectIdBytes::try_new(vec![0u8; 32]).expect("valid subject bytes"),
        )
        .expect("valid descriptor");

        let deposit_log = DepositLog::new(descriptor.encode_to_varvec(), 50_000_000);

        let log_entry =
            strata_asm_manifest_types::AsmLogEntry::from_log(&deposit_log).expect("should encode");
        let raw_bytes = log_entry.into_bytes();

        // Verify we can construct the tag with debug subprotocol params
        let tag = TagDataRef::new(DEBUG_SUBPROTOCOL_ID, MOCK_ASM_LOG_TX_TYPE, &raw_bytes)
            .expect("should build tag");

        assert_eq!(tag.subproto_id(), DEBUG_SUBPROTOCOL_ID);
        assert_eq!(tag.tx_type(), MOCK_ASM_LOG_TX_TYPE);
        assert_eq!(tag.aux_data(), raw_bytes.as_slice());
    }
}
