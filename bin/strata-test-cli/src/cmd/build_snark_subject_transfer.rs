//! CLI command for building a snark account subject-transfer transaction.

use argh::FromArgs;
use strata_acct_types::{AccountId, Hash, SubjectId};
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::mock_ee::subject_transfer;

/// Build a snark account subject-transfer transaction JSON for strata_submitTransaction.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "build-snark-subject-transfer")]
pub struct BuildSnarkSubjectTransferArgs {
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

    /// destination EE account ID (hex, 32 bytes)
    #[argh(option)]
    pub dest_account: String,

    /// source subject ID (hex, 32 bytes)
    #[argh(option)]
    pub source_subject: String,

    /// destination subject ID (hex, 32 bytes)
    #[argh(option)]
    pub dest_subject: String,

    /// transfer amount in satoshis
    #[argh(option)]
    pub amount: u64,

    /// subject-transfer metadata bytes (hex, default empty)
    #[argh(option, default = "String::new()")]
    pub transfer_data: String,
}

pub(crate) fn build_snark_subject_transfer(
    args: BuildSnarkSubjectTransferArgs,
) -> Result<(), DisplayedError> {
    let target = AccountId::new(parse_hex_32("target", &args.target)?);
    let inner_state = Hash::from(parse_hex_32("inner-state", &args.inner_state)?);
    let dest_account = AccountId::new(parse_hex_32("dest-account", &args.dest_account)?);
    let source_subject = SubjectId::new(parse_hex_32("source-subject", &args.source_subject)?);
    let dest_subject = SubjectId::new(parse_hex_32("dest-subject", &args.dest_subject)?);
    let transfer_data = parse_hex_var("transfer-data", &args.transfer_data)?;

    let json = subject_transfer::build_snark_subject_transfer_json(
        target,
        args.seq_no,
        inner_state,
        args.next_inbox_idx,
        dest_account,
        source_subject,
        dest_subject,
        transfer_data,
        args.amount,
    )
    .internal_error("failed to build subject transfer transaction")?;

    println!(
        "{}",
        serde_json::to_string(&json).expect("json serialization")
    );
    Ok(())
}

fn parse_hex_32(field: &str, hex_str: &str) -> Result<[u8; 32], DisplayedError> {
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(stripped)
        .map_err(|e| format!("invalid hex for {field}: {e}"))
        .internal_error("hex parse")?;

    bytes
        .try_into()
        .map_err(|v: Vec<u8>| format!("{field}: expected 32 bytes, got {}", v.len()))
        .internal_error("length check")
}

fn parse_hex_var(field: &str, hex_str: &str) -> Result<Vec<u8>, DisplayedError> {
    let stripped = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    hex::decode(stripped)
        .map_err(|e| format!("invalid hex for {field}: {e}"))
        .internal_error("hex parse")
}
