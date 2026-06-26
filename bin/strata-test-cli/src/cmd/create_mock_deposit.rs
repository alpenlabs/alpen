//! CLI command for creating a mock deposit via the debug subprotocol.

use argh::FromArgs;
use bdk_bitcoind_rpc::bitcoincore_rpc::RpcApi;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_identifiers::SubjectIdBytes;

use crate::{bridge::types::BitcoinDConfig, mock_ee::deposit, taproot::new_bitcoind_client};

/// Create a mock deposit transaction via the debug subprotocol.
///
/// Injects a DepositLog into the ASM by constructing a Bitcoin transaction
/// with a debug subprotocol (ID 255) MockAsmLog OP_RETURN payload.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "create-mock-deposit")]
pub struct CreateMockDepositArgs {
    /// snark account serial number (u32)
    #[argh(option)]
    pub account_serial: u32,

    /// deposit amount in satoshis
    #[argh(option)]
    pub amount: u64,

    /// destination subject ID (hex, 32 bytes; defaults to all zeroes)
    #[argh(option)]
    pub subject: Option<String>,

    /// bitcoin RPC URL
    #[argh(option)]
    pub btc_url: String,

    /// bitcoin RPC username
    #[argh(option)]
    pub btc_user: String,

    /// bitcoin RPC password
    #[argh(option)]
    pub btc_password: String,
}

pub(crate) fn create_mock_deposit(args: CreateMockDepositArgs) -> Result<(), DisplayedError> {
    let bitcoind_config = BitcoinDConfig {
        bitcoind_url: args.btc_url.clone(),
        bitcoind_user: args.btc_user.clone(),
        bitcoind_password: args.btc_password.clone(),
    };

    let subject = args
        .subject
        .as_deref()
        .map(parse_subject)
        .transpose()?
        .unwrap_or_else(|| SubjectIdBytes::try_new(vec![0u8; 32]).expect("valid subject bytes"));

    let tx_bytes =
        deposit::create_mock_deposit_tx(args.account_serial, subject, args.amount, bitcoind_config)
            .internal_error("failed to create mock deposit transaction")?;

    // Broadcast the transaction via bitcoind RPC
    let client = new_bitcoind_client(
        &args.btc_url,
        None,
        Some(&args.btc_user),
        Some(&args.btc_password),
    )
    .internal_error("failed to create bitcoind RPC client")?;

    let raw_hex = hex::encode(&tx_bytes);
    let txid: String = client
        .call("sendrawtransaction", &[serde_json::Value::String(raw_hex)])
        .map_err(|e| format!("failed to broadcast transaction: {e}"))
        .internal_error("broadcast failed")?;

    println!("{txid}");
    Ok(())
}

fn parse_subject(subject: &str) -> Result<SubjectIdBytes, DisplayedError> {
    let stripped = subject.strip_prefix("0x").unwrap_or(subject);
    let bytes = hex::decode(stripped)
        .map_err(|e| format!("invalid subject hex: {e}"))
        .internal_error("subject hex parse")?;

    SubjectIdBytes::try_new(bytes)
        .ok_or_else(|| "subject must fit within 32 bytes".to_owned())
        .internal_error("subject length")
}
