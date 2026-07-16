//! Admin commands operating on the EE chunk-receipt and acct-proof trees.
//!
//! Chunk receipts are keyed by the opaque chunk-task key (`Vec<u8>`),
//! matching the `paas::ReceiptStore` shape. Acct proofs are keyed by
//! [`alpen_ee_common::BatchId`], which the CLI parses from a
//! `<prev_block_hex>:<last_block_hex>` literal — the same shape
//! `BatchId::Display` emits, so operators can copy the `%batch_id` value
//! straight out of alpen-client logs.

use alpen_ee_common::{
    decode_chunk_task_key, encode_batch_task_key, Batch, BatchId, BatchStatus, BatchStorage, Chunk,
    ChunkId, ChunkStatus, ChunkStorage,
};
use alpen_ee_database::{EeNodeStorage, EeProverDbSled};
use argh::FromArgs;
use strata_acct_types::Hash;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_types::prover_task::ProverTaskDatabase;
use strata_primitives::buf::Buf32;

use crate::{
    cli::OutputFormat,
    cmd::prover_task_common::{parse_task_key, print_force_hint},
    output::{
        ee_receipts::{
            DeletedEeReceiptInfo, EeChunkReproofInfo, EeChunkReproofMutation, EeReceiptInfo,
        },
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

/// Invalidate a stored chunk proof and every dependent local proof artifact.
///
/// Use case: drop a stale receipt after a guest-program upgrade so the
/// chunk prover re-proves it.
///
/// The command deletes the chunk receipt and task, resets the chunk to
/// [`ChunkStatus::Sealed`], removes the owning batch's acct proof and task, and
/// moves a `ProofPending` or `ProofReady` batch back to `DaComplete`. The normal
/// lifecycle then re-proves the chunk and gates the replacement acct proof on
/// the new receipt.
///
/// Run with the node down. The cross-tree mutation is not transactional, but
/// every step is idempotent and the command reconstructs the same owning batch
/// from the persisted chunk row, so re-running after a partial failure is safe.
/// Dry-run unless `--force` is passed.
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
/// `get_proof_by_id` lookups miss instead of dangling. This is a low-level
/// proof-store operation: it does not reset the acct task or batch lifecycle
/// status and therefore does not schedule re-proving. Use the coupled
/// `ee-delete-chunk-receipt` command when invalidating a chunk and its dependent
/// acct proof. Dry-run unless `--force` is passed.
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

pub(crate) async fn ee_delete_chunk_receipt(
    storage: &EeNodeStorage,
    db: &EeProverDbSled,
    args: EeDeleteChunkReceiptArgs,
) -> Result<(), DisplayedError> {
    let key = parse_task_key(&args.key_hex)?;
    let chunk_id = decode_chunk_task_key(&key).map_err(|e| {
        DisplayedError::UserError(
            "Expected a canonical encoded chunk task key".to_string(),
            Box::new(e),
        )
    })?;

    let (chunk, chunk_status) = storage
        .get_chunk_by_id(chunk_id)
        .await
        .internal_error("Failed to read EE chunk for receipt invalidation")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "Chunk task key does not identify a persisted EE chunk".to_string(),
                Box::new(chunk_id_string(chunk_id)),
            )
        })?;
    let (batch, batch_status) = storage
        .get_batch_by_idx(chunk.batch_idx())
        .await
        .internal_error("Failed to read owning EE batch for receipt invalidation")?
        .ok_or_else(|| {
            DisplayedError::UserError(
                "Owning EE batch is missing for chunk receipt invalidation".to_string(),
                Box::new(chunk.batch_idx()),
            )
        })?;
    let batch_id = batch.id();
    if !chunk_belongs_to_batch(&chunk, &batch) {
        return Err(DisplayedError::UserError(
            "Chunk block range does not belong to the current batch at its recorded index"
                .to_string(),
            Box::new(format!(
                "chunk={} batch_idx={} current_batch={batch_id}",
                chunk_id_string(chunk_id),
                chunk.batch_idx()
            )),
        ));
    }
    if let Some(linked_chunks) = storage
        .get_batch_chunks(batch_id)
        .await
        .internal_error("Failed to read owning batch chunk links")?
    {
        if !linked_chunks.contains(&chunk_id) {
            return Err(DisplayedError::UserError(
                "Chunk is not linked to the current batch at its recorded index".to_string(),
                Box::new(format!(
                    "chunk={} batch_idx={} current_batch={batch_id}",
                    chunk_id_string(chunk_id),
                    chunk.batch_idx()
                )),
            ));
        }
    }
    let acct_task_key = encode_batch_task_key(batch_id);

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
    let acct_proof_existed = db
        .has_acct_proof(batch_id)
        .internal_error("Failed to read dependent EE acct proof")?;
    let acct_task_existed = db
        .get_task(acct_task_key.clone())
        .internal_error("Failed to read dependent EE acct prover task")?
        .is_some();

    let batch_status_reset = match &batch_status {
        BatchStatus::ProofPending { da } | BatchStatus::ProofReady { da, .. } => {
            Some(BatchStatus::DaComplete { da: da.clone() })
        }
        BatchStatus::Genesis
        | BatchStatus::Sealed
        | BatchStatus::DaPending { .. }
        | BatchStatus::DaComplete { .. } => None,
    };

    let mut report = EeChunkReproofInfo {
        address: args.key_hex,
        kind: "chunk",
        dry_run: !args.force,
        existed,
        task_existed,
        chunk_id: chunk_id_string(chunk_id),
        chunk_status: chunk_status_name(&chunk_status),
        batch_id: batch_id.to_string(),
        batch_status: batch_status_name(&batch_status),
        acct_proof_existed,
        acct_task_existed,
        mutation: EeChunkReproofMutation::default(),
    };

    if !args.force {
        output(&report, args.output_format)?;
        print_force_hint();
        return Ok(());
    }

    // Delete proof artifacts before changing lifecycle status. If an
    // intermediate mutation fails, a retry reconstructs the same plan from
    // the chunk and batch rows and repeats the idempotent deletes.
    report.mutation.receipt_deleted = db
        .delete_chunk_receipt(&key)
        .internal_error("Failed to delete chunk receipt")?;
    report.mutation.chunk_task_deleted = db
        .delete_task(key.clone())
        .internal_error("Failed to delete chunk prover task")?;
    report.mutation.acct_proof_deleted = db
        .delete_acct_proof(batch_id)
        .internal_error("Failed to delete dependent EE acct proof")?;
    report.mutation.acct_task_deleted = db
        .delete_task(acct_task_key)
        .internal_error("Failed to delete dependent EE acct prover task")?;

    if !matches!(chunk_status, ChunkStatus::Sealed) {
        storage
            .update_chunk_status(chunk_id, ChunkStatus::Sealed)
            .await
            .internal_error("Failed to reset EE chunk status for re-proof")?;
        report.mutation.chunk_status_reset = true;
    }

    if let Some(status) = batch_status_reset {
        storage
            .update_batch_status(batch_id, status)
            .await
            .internal_error("Failed to reset owning EE batch status for re-proof")?;
        report.mutation.batch_status_reset = true;
    }

    report.dry_run = false;
    output(&report, args.output_format)
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

fn chunk_id_string(chunk_id: ChunkId) -> String {
    format!("{:x}:{:x}", chunk_id.prev_block(), chunk_id.last_block())
}

/// Returns true when the chunk's ordered block range is contained in the batch.
///
/// A chunk can be unlinked briefly while its batch is still being assembled,
/// so block-range membership is the primary ownership check. Persisted links,
/// when present, are checked separately by the command.
fn chunk_belongs_to_batch(chunk: &Chunk, batch: &Batch) -> bool {
    let mut previous_block_seen = chunk.prev_block() == batch.prev_block();
    for block in batch.blocks_iter() {
        if previous_block_seen && block == chunk.last_block() {
            return true;
        }
        if block == chunk.prev_block() {
            previous_block_seen = true;
        }
    }
    false
}

fn chunk_status_name(status: &ChunkStatus) -> &'static str {
    match status {
        ChunkStatus::Sealed => "sealed",
        ChunkStatus::ProofPending(_) => "proof_pending",
        ChunkStatus::ProofReady(_) => "proof_ready",
    }
}

fn batch_status_name(status: &BatchStatus) -> &'static str {
    match status {
        BatchStatus::Genesis => "genesis",
        BatchStatus::Sealed => "sealed",
        BatchStatus::DaPending { .. } => "da_pending",
        BatchStatus::DaComplete { .. } => "da_complete",
        BatchStatus::ProofPending { .. } => "proof_pending",
        BatchStatus::ProofReady { .. } => "proof_ready",
    }
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

    #[test]
    fn chunk_batch_membership_requires_an_ordered_contained_range() {
        let prev = Hash::from([1u8; 32]);
        let middle = Hash::from([2u8; 32]);
        let last = Hash::from([3u8; 32]);
        let other = Hash::from([4u8; 32]);
        let batch = Batch::new(1, prev, last, 2, vec![middle]).unwrap();

        let first_chunk = Chunk::new(0, prev, middle, 1, 1, Vec::new());
        let second_chunk = Chunk::new(1, middle, last, 2, 1, Vec::new());
        let orphan = Chunk::new(2, prev, other, 2, 1, Vec::new());

        assert!(chunk_belongs_to_batch(&first_chunk, &batch));
        assert!(chunk_belongs_to_batch(&second_chunk, &batch));
        assert!(!chunk_belongs_to_batch(&orphan, &batch));
    }

    /// Invalidating a chunk receipt must reset the full chunk -> acct proof
    /// dependency chain; a dry run must leave every row and status intact.
    #[tokio::test]
    async fn ee_delete_chunk_receipt_resets_dependent_proof_chain() {
        use alpen_ee_database::init_db_storage;
        use strata_paas::{TaskRecordData, TaskStatus};
        use tokio::runtime::Handle;
        use zkaleido::{
            ProgramId, Proof, ProofMetadata, ProofReceipt, ProofReceiptWithMetadata, ProofType,
            PublicValues, ZkVm,
        };

        let datadir = tempfile::tempdir().unwrap();
        let databases = init_db_storage(datadir.path(), 2).unwrap();
        let storage = databases.node_storage(Handle::current());
        let db = databases.prover_db();

        let genesis_hash = Hash::from([1u8; 32]);
        let batch_last = Hash::from([2u8; 32]);
        let genesis = Batch::new_genesis_batch(genesis_hash, 0).unwrap();
        storage.save_genesis_batch(genesis).await.unwrap();

        let batch = Batch::new(1, genesis_hash, batch_last, 1, Vec::new()).unwrap();
        let batch_id = batch.id();
        storage.save_next_batch(batch).await.unwrap();

        let chunk = Chunk::new(0, genesis_hash, batch_last, 1, 1, Vec::new());
        let chunk_id = chunk.id();
        storage.save_next_chunk(chunk).await.unwrap();
        storage
            .set_batch_chunks(batch_id, vec![chunk_id])
            .await
            .unwrap();
        storage
            .update_chunk_status(chunk_id, ChunkStatus::ProofReady(batch_last))
            .await
            .unwrap();
        storage
            .update_batch_status(
                batch_id,
                BatchStatus::ProofReady {
                    da: Vec::new(),
                    proof: batch_last,
                },
            )
            .await
            .unwrap();

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

        let key = alpen_ee_common::encode_chunk_task_key(chunk_id);
        let acct_key = encode_batch_task_key(batch_id);
        db.put_chunk_receipt(key.clone(), receipt.clone()).unwrap();
        db.insert_task(key.clone(), TaskRecordData::new(TaskStatus::Completed))
            .unwrap();
        db.put_acct_proof(batch_id, receipt).unwrap();
        db.insert_task(acct_key.clone(), TaskRecordData::new(TaskStatus::Completed))
            .unwrap();

        let key_hex = hex::encode(&key);

        // Dry run: every proof artifact and lifecycle status survives.
        ee_delete_chunk_receipt(
            &storage,
            db.as_ref(),
            EeDeleteChunkReceiptArgs {
                key_hex: key_hex.clone(),
                force: false,
                output_format: OutputFormat::Porcelain,
            },
        )
        .await
        .unwrap();
        assert!(db.get_chunk_receipt(&key).unwrap().is_some());
        assert!(db.get_task(key.clone()).unwrap().is_some());
        assert!(db.has_acct_proof(batch_id).unwrap());
        assert!(db.get_task(acct_key.clone()).unwrap().is_some());
        assert!(matches!(
            storage.get_chunk_by_id(chunk_id).await.unwrap().unwrap().1,
            ChunkStatus::ProofReady(_)
        ));
        assert!(matches!(
            storage.get_batch_by_id(batch_id).await.unwrap().unwrap().1,
            BatchStatus::ProofReady { .. }
        ));

        // Force: chunk and acct artifacts are removed and both lifecycles are
        // placed before proof submission.
        ee_delete_chunk_receipt(
            &storage,
            db.as_ref(),
            EeDeleteChunkReceiptArgs {
                key_hex: key_hex.clone(),
                force: true,
                output_format: OutputFormat::Porcelain,
            },
        )
        .await
        .unwrap();
        assert!(db.get_chunk_receipt(&key).unwrap().is_none());
        assert!(db.get_task(key.clone()).unwrap().is_none());
        assert!(!db.has_acct_proof(batch_id).unwrap());
        assert!(db.get_task(acct_key).unwrap().is_none());
        assert!(matches!(
            storage.get_chunk_by_id(chunk_id).await.unwrap().unwrap().1,
            ChunkStatus::Sealed
        ));
        assert!(matches!(
            storage.get_batch_by_id(batch_id).await.unwrap().unwrap().1,
            BatchStatus::DaComplete { da } if da.is_empty()
        ));

        // Every mutation is idempotent, so the operator can rerun after an
        // unknown partial failure while the node remains stopped.
        ee_delete_chunk_receipt(
            &storage,
            db.as_ref(),
            EeDeleteChunkReceiptArgs {
                key_hex,
                force: true,
                output_format: OutputFormat::Porcelain,
            },
        )
        .await
        .unwrap();
    }
}
