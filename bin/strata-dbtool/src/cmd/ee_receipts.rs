//! Admin commands operating on the EE chunk-receipt and acct-proof trees.
//!
//! Chunk receipts are keyed by the opaque chunk-task key (`Vec<u8>`),
//! matching the `paas::ReceiptStore` shape. Acct proofs are keyed by
//! [`alpen_ee_common::BatchId`], which the CLI parses from a
//! `<prev_block_hex>:<last_block_hex>` literal — the same shape
//! `BatchId::Display` emits, so operators can copy the `%batch_id` value
//! straight out of alpen-client logs.

use alpen_ee_common::BatchId;
use alpen_ee_database::EeProverDbSled;
use argh::FromArgs;
use strata_acct_types::Hash;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_primitives::buf::Buf32;

use crate::{
    cli::OutputFormat,
    cmd::prover_task_common::{parse_task_key, print_force_hint},
    output::{
        ee_receipts::{DeletedEeReceiptInfo, EeReceiptInfo},
        output,
    },
};

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-get-chunk-receipt")]
/// Fetch a stored chunk-proof receipt by its task key.
pub(crate) struct EeGetChunkReceiptArgs {
    /// hex-encoded chunk task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-delete-chunk-receipt")]
/// Delete a stored chunk-proof receipt.
///
/// Use case: drop a stale receipt after a guest-program upgrade so the
/// next chunk-prover run re-proves it. Dry-run unless `--force` is passed.
pub(crate) struct EeDeleteChunkReceiptArgs {
    /// hex-encoded chunk task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-get-acct-proof")]
/// Fetch the stored acct/batch proof for a [`BatchId`].
pub(crate) struct EeGetAcctProofArgs {
    /// batch id as "<prev_block_hex>:<last_block_hex>" (each 32 bytes)
    #[argh(positional)]
    pub(crate) batch_id: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-delete-acct-proof")]
/// Delete the stored acct/batch proof for a [`BatchId`].
///
/// Also clears the secondary `ProofId → BatchId` index so future
/// `get_proof_by_id` lookups miss instead of dangling. Dry-run unless
/// `--force` is passed.
pub(crate) struct EeDeleteAcctProofArgs {
    /// batch id as "<prev_block_hex>:<last_block_hex>" (each 32 bytes)
    #[argh(positional)]
    pub(crate) batch_id: String,

    /// force execution (without this flag, only a dry run is performed)
    #[argh(switch, short = 'f')]
    pub(crate) force: bool,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Parses `"<prev_block_hex>:<last_block_hex>"` into a [`BatchId`].
///
/// Each hex half is exactly 32 bytes (64 hex chars), with an optional
/// `0x` prefix on either half. This is the exact shape `BatchId::Display`
/// emits, so a `%batch_id` value copied from alpen-client logs round-trips
/// through the parser.
pub(crate) fn parse_batch_id(s: &str) -> Result<BatchId, DisplayedError> {
    let (prev, last) = s.split_once(':').ok_or_else(|| {
        DisplayedError::UserError(
            "Expected batch id as <prev_block_hex>:<last_block_hex>".to_string(),
            Box::new(s.to_string()),
        )
    })?;
    let prev_hash = parse_hash(prev)?;
    let last_hash = parse_hash(last)?;
    Ok(BatchId::from_parts(prev_hash, last_hash))
}

fn parse_hash(s: &str) -> Result<Hash, DisplayedError> {
    let trimmed = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(trimmed).map_err(|e| {
        DisplayedError::UserError("Invalid hex-encoded block hash".to_string(), Box::new(e))
    })?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
        DisplayedError::UserError(
            format!(
                "Block hash must be exactly 32 bytes (got {} bytes)",
                bytes.len()
            ),
            Box::new(s.to_string()),
        )
    })?;
    Ok(Buf32(arr))
}

pub(crate) fn ee_get_chunk_receipt(
    db: &EeProverDbSled,
    args: EeGetChunkReceiptArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let receipt = db
        .get_chunk_receipt(&key)
        .internal_error("Failed to read chunk receipt")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No chunk receipt stored for task key".to_string(),
                Box::new(args.key_hex.clone()),
            )
        })?;

    let info = EeReceiptInfo::from_receipt(args.key_hex, "chunk", &receipt);
    output(&info, args.output_format)
}

pub(crate) fn ee_delete_chunk_receipt(
    db: &EeProverDbSled,
    args: EeDeleteChunkReceiptArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;

    // Resolve existence up front so the dry run still emits the same
    // structured `existed` field operators check against.
    let existed = db
        .get_chunk_receipt(&key)
        .internal_error("Failed to read chunk receipt")?
        .is_some();

    if !args.force {
        let ack = DeletedEeReceiptInfo {
            address: args.key_hex,
            kind: "chunk",
            existed,
        };
        output(&ack, args.output_format)?;
        print_force_hint();
        return Ok(());
    }

    let actually_existed = db
        .delete_chunk_receipt(&key)
        .internal_error("Failed to delete chunk receipt")?;

    let ack = DeletedEeReceiptInfo {
        address: args.key_hex,
        kind: "chunk",
        existed: actually_existed,
    };
    output(&ack, args.output_format)
}

pub(crate) fn ee_get_acct_proof(
    db: &EeProverDbSled,
    args: EeGetAcctProofArgs,
) -> Result<(), DisplayedError> {
    let batch_id = parse_batch_id(&args.batch_id)?;
    let receipt = db
        .get_acct_proof(batch_id)
        .internal_error("Failed to read acct proof")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No acct proof stored for batch id".to_string(),
                Box::new(args.batch_id.clone()),
            )
        })?;

    let info = EeReceiptInfo::from_receipt(args.batch_id, "acct", &receipt);
    output(&info, args.output_format)
}

pub(crate) fn ee_delete_acct_proof(
    db: &EeProverDbSled,
    args: EeDeleteAcctProofArgs,
) -> Result<(), DisplayedError> {
    let batch_id = parse_batch_id(&args.batch_id)?;

    // Resolve existence up front so the dry run still emits the same
    // structured `existed` field operators check against.
    let existed = db
        .has_acct_proof(batch_id)
        .internal_error("Failed to read acct proof")?;

    if !args.force {
        let ack = DeletedEeReceiptInfo {
            address: args.batch_id,
            kind: "acct",
            existed,
        };
        output(&ack, args.output_format)?;
        print_force_hint();
        return Ok(());
    }

    let actually_existed = db
        .delete_acct_proof(batch_id)
        .internal_error("Failed to delete acct proof")?;

    let ack = DeletedEeReceiptInfo {
        address: args.batch_id,
        kind: "acct",
        existed: actually_existed,
    };
    output(&ack, args.output_format)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_batch_id_accepts_two_32_byte_halves() {
        let prev = "11".repeat(32);
        let last = "22".repeat(32);
        let bid = parse_batch_id(&format!("{prev}:{last}")).unwrap();
        assert_eq!(bid.prev_block().0, [0x11u8; 32]);
        assert_eq!(bid.last_block().0, [0x22u8; 32]);
    }

    #[test]
    fn parse_batch_id_accepts_0x_prefix_on_each_half() {
        let prev = format!("0x{}", "aa".repeat(32));
        let last = format!("0x{}", "bb".repeat(32));
        let bid = parse_batch_id(&format!("{prev}:{last}")).unwrap();
        assert_eq!(bid.prev_block().0, [0xaau8; 32]);
        assert_eq!(bid.last_block().0, [0xbbu8; 32]);
    }

    #[test]
    fn parse_batch_id_rejects_missing_colon() {
        assert!(parse_batch_id(&"11".repeat(64)).is_err());
    }

    #[test]
    fn parse_batch_id_rejects_wrong_length_halves() {
        // 31 bytes — short
        let prev = "11".repeat(31);
        let last = "22".repeat(32);
        assert!(parse_batch_id(&format!("{prev}:{last}")).is_err());

        // 33 bytes — long
        let prev = "11".repeat(33);
        let last = "22".repeat(32);
        assert!(parse_batch_id(&format!("{prev}:{last}")).is_err());
    }

    #[test]
    fn parse_batch_id_rejects_non_hex() {
        let last = "22".repeat(32);
        assert!(parse_batch_id(&format!("not-hex:{last}")).is_err());
    }
}
