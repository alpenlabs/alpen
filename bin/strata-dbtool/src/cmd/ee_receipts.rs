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
use strata_db_types::prover_task::ProverTaskDatabase;
use strata_primitives::buf::Buf32;

use crate::{
    cli::OutputFormat,
    cmd::prover_task_common::{parse_task_key, print_force_hint},
    output::{
        ee_receipts::{DeletedEeReceiptInfo, EeReceiptInfo},
        output,
    },
};

/// Fetch a stored chunk-proof receipt by its task key.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-get-chunk-receipt")]
pub(crate) struct EeGetChunkReceiptArgs {
    /// hex-encoded chunk task key
    #[argh(positional)]
    pub(crate) key_hex: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Delete a stored chunk-proof receipt and its companion prover task.
///
/// Use case: drop a stale receipt after a guest-program upgrade so the
/// chunk prover re-proves it.
///
/// Re-proving is driven by the PaaS *task* store, not by receipt presence:
/// a finished chunk task is `Completed`, and nothing re-runs a `Completed`
/// task. Deleting only the receipt would therefore never re-prove, and would
/// instead wedge the chunk in a `ProofReady`<->`ProofPending` oscillation
/// (the acct gate flips a receipt-less `ProofReady` chunk to `ProofPending`,
/// while the still-`Completed` task keeps reporting `Ready`). So this command
/// deletes both rows under the same key: the receipt and the task record.
///
/// Run with the node down. The two-tree
/// delete is not transactional; re-running is safe (both deletes are
/// idempotent). If the batch's acct proof already exists, the acct gate no
/// longer re-fires, so also run `ee-delete-acct-proof <prev>:<last>` to force
/// the chunk to re-prove. Dry-run unless `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-delete-chunk-receipt")]
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

/// Fetch the stored acct/batch proof for a [`BatchId`].
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-get-acct-proof")]
pub(crate) struct EeGetAcctProofArgs {
    /// batch id as "<prev_block_hex>:<last_block_hex>" (each 32 bytes)
    #[argh(positional)]
    pub(crate) batch_id: String,

    /// output format: "porcelain" (default) or "json"
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Delete the stored acct/batch proof for a [`BatchId`].
///
/// Also clears the secondary `ProofId → BatchId` index so future
/// `get_proof_by_id` lookups miss instead of dangling. Dry-run unless
/// `--force` is passed.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-delete-acct-proof")]
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

    // Resolve existence of both rows up front so the dry run reports exactly
    // what `--force` would delete. The receipt and the prover task share the
    // same key (the prover stores the receipt under the task key), so a single
    // `key_hex` drives both deletes.
    let existed = db
        .get_chunk_receipt(&key)
        .internal_error("Failed to read chunk receipt")?
        .is_some();
    let task_existed = db
        .get_task(key.clone())
        .internal_error("Failed to read chunk prover task")?
        .is_some();

    if !args.force {
        let ack = DeletedEeReceiptInfo {
            address: args.key_hex,
            kind: "chunk",
            existed,
            task_existed: Some(task_existed),
        };
        output(&ack, args.output_format)?;
        print_force_hint();
        return Ok(());
    }

    // Delete the receipt first, then the task record. Both are idempotent, so
    // re-running after a partial failure is safe. Deleting the task moves it
    // out of `Completed`, so the chunk lifecycle re-submits and re-proves.
    let actually_existed = db
        .delete_chunk_receipt(&key)
        .internal_error("Failed to delete chunk receipt")?;
    let task_actually_existed = db
        .delete_task(key)
        .internal_error("Failed to delete chunk prover task")?;

    let ack = DeletedEeReceiptInfo {
        address: args.key_hex,
        kind: "chunk",
        existed: actually_existed,
        task_existed: Some(task_actually_existed),
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
            task_existed: None,
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
        task_existed: None,
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

    /// `ee-delete-chunk-receipt --force` must delete BOTH the chunk receipt and
    /// its companion prover task record (so the chunk actually re-proves);
    /// a dry run must leave both intact.
    #[test]
    fn ee_delete_chunk_receipt_force_clears_receipt_and_task() {
        use std::sync::Arc;

        use alpen_ee_database::EeProverDbSled;
        use strata_db_store_sled::SledDbConfig;
        use strata_paas::{TaskRecordData, TaskStatus};
        use typed_sled::SledDb;
        use zkaleido::{
            ProgramId, Proof, ProofMetadata, ProofReceipt, ProofReceiptWithMetadata, ProofType,
            PublicValues, ZkVm,
        };

        let sled_db = sled::Config::new().temporary(true).open().unwrap();
        let typed = Arc::new(SledDb::new(sled_db).unwrap());
        let config = SledDbConfig::new_with_constant_backoff(2, 0);
        let db = EeProverDbSled::new(typed, config).unwrap();

        // Chunk task key (kind tag `b'c'` + payload); the receipt and the task
        // record are stored under the same key.
        let key = vec![b'c', 1, 2, 3, 4];
        let receipt = {
            let metadata = ProofMetadata::new(
                ZkVm::Native,
                ProgramId([1u8; 32]),
                "0.1".to_string(),
                ProofType::Groth16,
            );
            let r = ProofReceipt::new(Proof::new(vec![1, 2]), PublicValues::new(vec![3]));
            ProofReceiptWithMetadata::new(r, metadata)
        };
        db.put_chunk_receipt(key.clone(), receipt).unwrap();
        db.insert_task(key.clone(), TaskRecordData::new(TaskStatus::Completed))
            .unwrap();

        let key_hex = hex::encode(&key);

        // Dry run: both rows survive.
        ee_delete_chunk_receipt(
            &db,
            EeDeleteChunkReceiptArgs {
                key_hex: key_hex.clone(),
                force: false,
                output_format: OutputFormat::Porcelain,
            },
        )
        .unwrap();
        assert!(db.get_chunk_receipt(&key).unwrap().is_some());
        assert!(db.get_task(key.clone()).unwrap().is_some());

        // Force: both the receipt and the companion task record are gone.
        ee_delete_chunk_receipt(
            &db,
            EeDeleteChunkReceiptArgs {
                key_hex,
                force: true,
                output_format: OutputFormat::Porcelain,
            },
        )
        .unwrap();
        assert!(db.get_chunk_receipt(&key).unwrap().is_none());
        assert!(db.get_task(key).unwrap().is_none());
    }
}
