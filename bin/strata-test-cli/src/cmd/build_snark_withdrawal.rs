//! CLI command for building a snark account withdrawal transaction.

use argh::FromArgs;
use strata_acct_types::{AccountId, Hash};
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::mock_ee::withdrawal;

/// Build a snark account withdrawal transaction JSON for strata_submitTransaction.
///
/// Pure computation — no network calls. Outputs JSON to stdout.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "build-snark-withdrawal")]
pub struct BuildSnarkWithdrawalArgs {
    /// target account ID (hex, 32 bytes)
    #[argh(option)]
    pub target: String,

    /// sequence number
    #[argh(option)]
    pub seq_no: u64,

    /// inner state hash (hex, 32 bytes)
    #[argh(option)]
    pub inner_state: String,

    /// next inbox message index
    #[argh(option)]
    pub next_inbox_idx: u64,

    /// withdrawal destination descriptor (hex-encoded bytes)
    #[argh(option)]
    pub dest: String,

    /// withdrawal amount in satoshis
    #[argh(option)]
    pub amount: u64,

    /// operator fees in satoshis (default: 0)
    #[argh(option, default = "0")]
    pub fees: u32,
}

pub(crate) fn build_snark_withdrawal(args: BuildSnarkWithdrawalArgs) -> Result<(), DisplayedError> {
    let target_bytes = parse_hex_32("target", &args.target)?;
    let target = AccountId::new(target_bytes);

    let inner_state_bytes = parse_hex_32("inner-state", &args.inner_state)?;
    let inner_state = Hash::from(inner_state_bytes);

    let dest_bytes = parse_hex_var("dest", &args.dest)?;

    let json = withdrawal::build_snark_withdrawal_json(
        target,
        args.seq_no,
        inner_state,
        args.next_inbox_idx,
        dest_bytes,
        args.amount,
        args.fees,
    )
    .internal_error("failed to build withdrawal transaction")?;

    println!("{}", serde_json::to_string(&json).expect("json serialization"));
    Ok(())
}

/// Parses a hex string (with optional "0x" prefix) into a 32-byte array.
fn parse_hex_32(field: &str, hex_str: &str) -> Result<[u8; 32], DisplayedError> {
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(stripped)
        .map_err(|e| format!("invalid hex for {field}: {e}"))
        .internal_error("hex parse")?;

    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("{field}: expected 32 bytes, got {}", v.len()))
        .internal_error("length check")?;

    Ok(arr)
}

/// Parses a hex string (with optional "0x" prefix) into a variable-length byte vec.
fn parse_hex_var(field: &str, hex_str: &str) -> Result<Vec<u8>, DisplayedError> {
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    hex::decode(stripped)
        .map_err(|e| format!("invalid hex for {field}: {e}"))
        .internal_error("hex parse")
}
