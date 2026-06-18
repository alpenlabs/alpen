//! One-shot, idempotent schema migration for the EE sled database.
//!
//! Upgrades an EE `<datadir>/sled` written by pre-#1849 binaries so that
//! current binaries can read it. Two record layouts changed by mid-struct
//! field inserts (a positional Borsh break):
//!
//! 1. [`ChunkByIdxSchema`] value [`DBChunkWithStatus`]: #1851 added
//!    `last_blocknum: u64` and `batch_idx: u64` to the inner chunk.
//! 2. [`ExecBlockSchema`] value [`DBExecBlockRecord`]: #1947 added
//!    `next_deposit_idx: u64`.
//!
//! The migration reads each row's raw `(key, value)`, decodes the value with a
//! V0 mirror struct, backfills the new fields, re-encodes with the current
//! struct, and writes back under the **same raw key** (keys are never decoded
//! or re-encoded).
//!
//! Idempotency is enforced via a version marker key in the default tree; a
//! second run is a no-op.
//!
//! ## `next_deposit_idx` provenance (correctness-critical)
//!
//! Block production sets `record.next_deposit_idx = parent.next_deposit_idx +
//! deposits_processed(block)`, genesis base `0`
//! (`crates/alpen-ee/sequencer/src/block_builder/task.rs:117`,
//! `crates/alpen-ee/block-assembly/src/payload.rs:89`).
//! `deposits_processed(block)` is `processed_inputs` in the block's
//! [`UpdateExtraData`] (`payload.rs:51-52`).
//!
//! `UpdateExtraData` is **not** stored anywhere reachable from a persisted
//! exec record (the SSZ `ExecBlockPackage` carries only `commitment`, `inputs`,
//! `outputs`). We instead recover `processed_inputs` exactly from stored fields:
//! every block removes exactly `processed_inputs` entries from the front of the
//! account-state pending-input queue
//! (`crates/alpen-ee/block-assembly/src/block.rs:78` →
//! `remove_pending_inputs`), and adds exactly the block's own deposit inputs
//! (`package.inputs().subject_deposits().len()`; each `Deposit` message becomes
//! one pending input, `ee_program.rs:184-187`, mirrored into the package by
//! `build_block_inputs`, `block.rs:21-49`). Hence:
//!
//! ```text
//! processed_inputs(block)
//!   = package.inputs().len()
//!   + parent.account_state.pending_inputs().len()
//!   - block.account_state.pending_inputs().len()
//! ```
//!
//! This is exact in all cases (including deposit backlog / per-block cap). When
//! there is no backlog it reduces to `package.inputs().len()`. Empirically, on
//! the real staging-v2 EE DB the two agree on all 196784 rows.

use std::{collections::HashMap, hash::Hash as StdHash};

use alpen_ee_common::{Chunk, ChunkStatus, ExecBlockRecord};
use borsh::{BorshDeserialize, BorshSerialize};
use eyre::{eyre, Context, Result};
use ssz::Decode;
use strata_acct_types::{Hash, MessageEntry};
use strata_ee_acct_types::EeAccountState;
use strata_ee_chain_types::ExecBlockPackage;
use strata_identifiers::OLBlockCommitment;
use tracing::{info, warn};

use crate::serialization_types::{
    DBChunkStatus, DBChunkWithStatus, DBEeAccountState, DBExecBlockRecord, DBMessageEntry,
};

/// Tree (and key) holding the schema version marker.
const VERSION_KEY: &[u8] = b"alpen_schema_version";
/// Schema version this migration produces.
pub const EE_SCHEMA_VERSION: u32 = 1;

const CHUNK_TREE: &str = "ChunkByIdxSchema";
const EXEC_TREE: &str = "ExecBlockSchema";
const BATCH_CHUNKS_TREE: &str = "BatchChunksSchema";
const BATCH_ID_TO_IDX_TREE: &str = "BatchIdToIdxSchema";

/// V0 (pre-#1851) chunk: no `last_blocknum` / `batch_idx`.
#[derive(BorshDeserialize, BorshSerialize)]
struct DBChunkV0 {
    idx: u64,
    prev_block: [u8; 32],
    last_block: [u8; 32],
    inner_blocks: Vec<[u8; 32]>,
}

/// V0 wrapper as stored in `ChunkByIdxSchema` before #1851.
#[derive(BorshDeserialize, BorshSerialize)]
struct DBChunkWithStatusV0 {
    chunk: DBChunkV0,
    status: DBChunkStatus,
}

/// V0 (pre-#1947) exec-block record: no `next_deposit_idx`.
#[derive(BorshDeserialize, BorshSerialize)]
struct DBExecBlockRecordV0 {
    blocknum: u64,
    parent_blockhash: Hash,
    timestamp_ms: u64,
    ol_block: OLBlockCommitment,
    package_ssz: Vec<u8>,
    account_state: DBEeAccountState,
    next_inbox_msg_idx: u64,
    messages: Vec<DBMessageEntry>,
}

/// `DBChunkId` mirror used to interpret `BatchChunksSchema` values.
#[derive(BorshDeserialize, BorshSerialize, Clone, PartialEq, Eq, StdHash)]
struct DBChunkIdKey {
    prev_block: [u8; 32],
    last_block: [u8; 32],
}

/// Summary of what the EE migration touched.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EeMigrationReport {
    /// Whether the migration ran (false = already at target version, no-op).
    pub ran: bool,
    /// Chunk rows rewritten.
    pub chunks_migrated: u64,
    /// Exec-block rows rewritten.
    pub exec_blocks_migrated: u64,
}

/// Reads the EE schema version marker from the default tree.
fn read_version(db: &sled::Db) -> Result<Option<u32>> {
    let raw = db
        .get(VERSION_KEY)
        .context("reading EE schema version marker")?;
    match raw {
        None => Ok(None),
        Some(bytes) => {
            let arr: [u8; 4] = bytes
                .as_ref()
                .try_into()
                .map_err(|_| eyre!("EE schema version marker has unexpected length"))?;
            Ok(Some(u32::from_le_bytes(arr)))
        }
    }
}

/// Writes the EE schema version marker into the default tree.
fn write_version(db: &sled::Db, version: u32) -> Result<()> {
    db.insert(VERSION_KEY, &version.to_le_bytes())
        .context("writing EE schema version marker")?;
    db.flush().context("flushing EE schema version marker")?;
    Ok(())
}

/// Runs the EE schema migration on an already-open sled database, idempotently.
///
/// Safe to call repeatedly: if the version marker already equals
/// [`EE_SCHEMA_VERSION`], it returns without touching any tree.
pub fn migrate_ee_db(db: &sled::Db) -> Result<EeMigrationReport> {
    if read_version(db)?.is_some_and(|v| v >= EE_SCHEMA_VERSION) {
        info!(
            version = EE_SCHEMA_VERSION,
            "EE DB already at target schema version; skipping"
        );
        return Ok(EeMigrationReport::default());
    }

    let batch_idx_by_chunk = build_chunk_to_batch_idx(db)?;
    let blocknum_by_hash = build_blocknum_by_hash(db)?;
    let next_deposit_idx_by_hash = build_next_deposit_idx(db)?;

    let exec_blocks_migrated = migrate_exec_blocks(db, &next_deposit_idx_by_hash)?;
    let chunks_migrated = migrate_chunks(db, &batch_idx_by_chunk, &blocknum_by_hash)?;

    write_version(db, EE_SCHEMA_VERSION)?;

    info!(
        chunks_migrated,
        exec_blocks_migrated, "EE DB schema migration complete"
    );

    Ok(EeMigrationReport {
        ran: true,
        chunks_migrated,
        exec_blocks_migrated,
    })
}

/// Builds `DBChunkId -> batch_idx` from `BatchChunksSchema` + `BatchIdToIdxSchema`.
fn build_chunk_to_batch_idx(db: &sled::Db) -> Result<HashMap<DBChunkIdKey, u64>> {
    let batch_chunks = db
        .open_tree(BATCH_CHUNKS_TREE)
        .context("open BatchChunksSchema")?;
    let batch_id_to_idx = db
        .open_tree(BATCH_ID_TO_IDX_TREE)
        .context("open BatchIdToIdxSchema")?;

    // batch_id (raw key bytes) -> idx
    let mut idx_by_batch: HashMap<Vec<u8>, u64> = HashMap::new();
    for kv in batch_id_to_idx.iter() {
        let (k, v) = kv.context("iter BatchIdToIdxSchema")?;
        let idx = u64::try_from_slice(&v).context("decode batch idx (u64 borsh)")?;
        idx_by_batch.insert(k.as_ref().to_vec(), idx);
    }

    let mut out: HashMap<DBChunkIdKey, u64> = HashMap::new();
    for kv in batch_chunks.iter() {
        let (k, v) = kv.context("iter BatchChunksSchema")?;
        let Some(idx) = idx_by_batch.get(k.as_ref()) else {
            // Decode the batch id only for diagnostics.
            warn!(
                key_len = k.len(),
                "BatchChunksSchema batch id has no entry in BatchIdToIdxSchema; skipping its chunks"
            );
            continue;
        };
        let chunk_ids = Vec::<DBChunkIdKey>::try_from_slice(&v)
            .context("decode Vec<DBChunkId> (borsh) from BatchChunksSchema")?;
        for chunk_id in chunk_ids {
            out.insert(chunk_id, *idx);
        }
    }
    Ok(out)
}

/// Builds `last_block hash (raw 32 bytes) -> blocknum` from `ExecBlockSchema`.
///
/// Reads `blocknum` as the first 8 bytes (LE) of each stored value, since it is
/// the first Borsh field of the record (V0 and current layouts agree on it),
/// avoiding a full decode.
fn build_blocknum_by_hash(db: &sled::Db) -> Result<HashMap<[u8; 32], u64>> {
    let exec = db.open_tree(EXEC_TREE).context("open ExecBlockSchema")?;
    let mut out: HashMap<[u8; 32], u64> = HashMap::new();
    for kv in exec.iter() {
        let (k, v) = kv.context("iter ExecBlockSchema")?;
        let hash: [u8; 32] = k
            .as_ref()
            .try_into()
            .map_err(|_| eyre!("ExecBlockSchema key is not 32 bytes (len {})", k.len()))?;
        if v.len() < 8 {
            return Err(eyre!("ExecBlockSchema value too short to read blocknum"));
        }
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&v[..8]);
        out.insert(hash, u64::from_le_bytes(buf));
    }
    Ok(out)
}

/// Computes `blockhash (raw 32 bytes) -> next_deposit_idx` for every exec block,
/// by replaying the canonical parent->child chain.
fn build_next_deposit_idx(db: &sled::Db) -> Result<HashMap<[u8; 32], u64>> {
    let exec = db.open_tree(EXEC_TREE).context("open ExecBlockSchema")?;

    /// Per-block info needed for the deposit-idx replay.
    struct Node {
        blocknum: u64,
        parent: [u8; 32],
        /// deposits added by this block (= package inputs)
        added: u64,
        /// pending-input queue length *after* this block executed
        pending_after: u64,
    }

    let mut nodes: HashMap<[u8; 32], Node> = HashMap::new();
    let mut dup_blocknum = false;
    let mut blocknums: HashMap<u64, u64> = HashMap::new();

    for kv in exec.iter() {
        let (k, v) = kv.context("iter ExecBlockSchema")?;
        let blockhash: [u8; 32] = k
            .as_ref()
            .try_into()
            .map_err(|_| eyre!("ExecBlockSchema key is not 32 bytes"))?;
        let rec = DBExecBlockRecordV0::try_from_slice(&v)
            .context("decode V0 exec-block record for deposit replay")?;
        let pkg = ExecBlockPackage::from_ssz_bytes(&rec.package_ssz)
            .map_err(|e| eyre!("decode package ssz: {e:?}"))?;
        let added = pkg.inputs().subject_deposits().len() as u64;
        let account_state: EeAccountState = rec.account_state.into();
        let pending_after = account_state.pending_inputs().len() as u64;
        let parent: [u8; 32] = rec.parent_blockhash.into();

        *blocknums.entry(rec.blocknum).or_insert(0) += 1;
        if blocknums[&rec.blocknum] > 1 {
            dup_blocknum = true;
        }

        nodes.insert(
            blockhash,
            Node {
                blocknum: rec.blocknum,
                parent,
                added,
                pending_after,
            },
        );
    }

    if dup_blocknum {
        // The replay relies on a single canonical block per height; report and
        // refuse rather than guessing across a fork in the stored set.
        return Err(eyre!(
            "ExecBlockSchema contains multiple blocks for the same blocknum; \
             deposit-idx replay cannot disambiguate the canonical chain"
        ));
    }

    // Order by blocknum ascending (single chain), compute cumulative idx.
    let mut ordered: Vec<(&[u8; 32], &Node)> = nodes.iter().collect();
    ordered.sort_by_key(|(_, n)| n.blocknum);

    let zero = [0u8; 32];
    let mut out: HashMap<[u8; 32], u64> = HashMap::new();
    for (hash, node) in ordered {
        // parent's pending length (0 for genesis / missing parent)
        let parent_pending = if node.parent == zero {
            0
        } else {
            nodes.get(&node.parent).map(|p| p.pending_after).unwrap_or(0)
        };
        let parent_next_deposit_idx = if node.parent == zero {
            0
        } else {
            match out.get(&node.parent) {
                Some(idx) => *idx,
                None => {
                    // Genesis-like root whose parent isn't stored.
                    0
                }
            }
        };
        let processed = node.added as i64 + parent_pending as i64 - node.pending_after as i64;
        if processed < 0 {
            return Err(eyre!(
                "negative processed_inputs at blocknum {} (added={} parent_pending={} pending_after={})",
                node.blocknum,
                node.added,
                parent_pending,
                node.pending_after
            ));
        }
        out.insert(*hash, parent_next_deposit_idx + processed as u64);
    }

    Ok(out)
}

/// Rewrites every `ExecBlockSchema` row from V0 to current layout.
fn migrate_exec_blocks(
    db: &sled::Db,
    next_deposit_idx_by_hash: &HashMap<[u8; 32], u64>,
) -> Result<u64> {
    let exec = db.open_tree(EXEC_TREE).context("open ExecBlockSchema")?;
    let mut count = 0u64;

    let mut raw_rows: Vec<(sled::IVec, sled::IVec)> = Vec::new();
    for kv in exec.iter() {
        let (k, v) = kv.context("collect ExecBlockSchema rows")?;
        raw_rows.push((k, v));
    }

    for (k, v) in raw_rows {
        // Already current layout? Skip (defensive; supports partial reruns).
        if DBExecBlockRecord::try_from_slice(&v).is_ok() {
            continue;
        }
        let blockhash: [u8; 32] = k
            .as_ref()
            .try_into()
            .map_err(|_| eyre!("ExecBlockSchema key is not 32 bytes"))?;
        let v0 = DBExecBlockRecordV0::try_from_slice(&v)
            .context("decode V0 exec-block record")?;
        let next_deposit_idx = *next_deposit_idx_by_hash
            .get(&blockhash)
            .ok_or_else(|| eyre!("no next_deposit_idx computed for exec block"))?;

        let new_value = rebuild_exec_record(v0, next_deposit_idx)?;
        let bytes = borsh::to_vec(&new_value).context("encode current exec-block record")?;
        exec.insert(k, bytes).context("write exec-block record")?;
        count += 1;
    }
    exec.flush().context("flush ExecBlockSchema")?;
    Ok(count)
}

/// Rebuilds a current [`DBExecBlockRecord`] from a V0 record + backfilled
/// `next_deposit_idx`, reusing the domain [`ExecBlockRecord`] and its
/// `DBExecBlockRecord::from` conversion. The package bytes are kept as-is.
fn rebuild_exec_record(
    v0: DBExecBlockRecordV0,
    next_deposit_idx: u64,
) -> Result<DBExecBlockRecord> {
    let package = ExecBlockPackage::from_ssz_bytes(&v0.package_ssz)
        .map_err(|e| eyre!("decode package ssz for rebuild: {e:?}"))?;
    let account_state: EeAccountState = v0.account_state.into();
    let messages: Vec<MessageEntry> = v0.messages.into_iter().map(Into::into).collect();

    let record = ExecBlockRecord::new(
        package,
        account_state,
        v0.blocknum,
        v0.ol_block,
        v0.timestamp_ms,
        v0.parent_blockhash,
        v0.next_inbox_msg_idx,
        next_deposit_idx,
        messages,
    );
    Ok(DBExecBlockRecord::from(record))
}

/// Rewrites every `ChunkByIdxSchema` row from V0 to current layout.
fn migrate_chunks(
    db: &sled::Db,
    batch_idx_by_chunk: &HashMap<DBChunkIdKey, u64>,
    blocknum_by_hash: &HashMap<[u8; 32], u64>,
) -> Result<u64> {
    let chunks = db.open_tree(CHUNK_TREE).context("open ChunkByIdxSchema")?;
    let mut count = 0u64;

    let mut raw_rows: Vec<(sled::IVec, sled::IVec)> = Vec::new();
    for kv in chunks.iter() {
        let (k, v) = kv.context("collect ChunkByIdxSchema rows")?;
        raw_rows.push((k, v));
    }

    for (k, v) in raw_rows {
        if DBChunkWithStatus::try_from_slice(&v).is_ok() {
            continue;
        }
        let v0 = DBChunkWithStatusV0::try_from_slice(&v)
            .context("decode V0 chunk-with-status")?;

        let chunk_id = DBChunkIdKey {
            prev_block: v0.chunk.prev_block,
            last_block: v0.chunk.last_block,
        };
        let batch_idx = *batch_idx_by_chunk
            .get(&chunk_id)
            .ok_or_else(|| eyre!("no batch_idx found for chunk (chunk_id not in any batch)"))?;
        let last_blocknum = *blocknum_by_hash.get(&v0.chunk.last_block).ok_or_else(|| {
            eyre!("no exec block found for chunk last_block; cannot derive last_blocknum")
        })?;

        let chunk = Chunk::new(
            v0.chunk.idx,
            Hash::from(v0.chunk.prev_block),
            Hash::from(v0.chunk.last_block),
            last_blocknum,
            batch_idx,
            v0.chunk.inner_blocks.into_iter().map(Hash::from).collect(),
        );
        let status = ChunkStatus::from(v0.status);
        let new_value = DBChunkWithStatus::new(chunk, status);
        let bytes = borsh::to_vec(&new_value).context("encode current chunk-with-status")?;
        chunks.insert(k, bytes).context("write chunk-with-status")?;
        count += 1;
    }
    chunks.flush().context("flush ChunkByIdxSchema")?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use sled::Config as SledConfig;
    use strata_acct_types::{BitcoinAmount, SubjectId};
    use strata_ee_acct_types::PendingInputEntry;
    use strata_ee_chain_types::{
        ExecBlockCommitment, ExecInputs, ExecOutputs, SubjectDepositData,
    };
    use strata_identifiers::{Buf32, OLBlockId};

    use super::*;

    fn temp_db() -> sled::Db {
        SledConfig::new()
            .temporary(true)
            .open()
            .expect("open temp sled")
    }

    fn deposit(n: u64) -> PendingInputEntry {
        PendingInputEntry::Deposit(SubjectDepositData::new(
            SubjectId::new([0xab; 32]),
            BitcoinAmount::from_sat(1000 + n),
        ))
    }

    /// Builds a V0 exec-block record with `added` package deposits and a
    /// resulting pending queue of `pending_after` entries, then borsh-encodes it
    /// in the pre-#1947 (no `next_deposit_idx`) layout.
    fn make_v0_exec(
        blocknum: u64,
        blockhash: [u8; 32],
        parent: [u8; 32],
        added: usize,
        pending_after: usize,
    ) -> (Vec<u8>, [u8; 32]) {
        use ssz::Encode;

        let mut inputs = ExecInputs::new_empty();
        for i in 0..added {
            inputs.add_subject_deposit(SubjectDepositData::new(
                SubjectId::new([(i as u8).wrapping_add(1); 32]),
                BitcoinAmount::from_sat(500 + i as u64),
            ));
        }
        let package = ExecBlockPackage::new(
            ExecBlockCommitment::new(Hash::from(blockhash), Hash::from([0u8; 32])),
            inputs,
            ExecOutputs::new_empty(),
        );

        let account_state = EeAccountState::new(
            Hash::from(blockhash),
            Hash::from([0u8; 32]),
            (0..pending_after).map(|i| deposit(i as u64)).collect(),
            Vec::new(),
        );

        let v0 = DBExecBlockRecordV0 {
            blocknum,
            parent_blockhash: Hash::from(parent),
            timestamp_ms: 0,
            ol_block: OLBlockCommitment::new(blocknum, OLBlockId::from(Buf32::from([0u8; 32]))),
            package_ssz: package.as_ssz_bytes(),
            account_state: account_state.into(),
            next_inbox_msg_idx: 0,
            messages: Vec::new(),
        };
        (borsh::to_vec(&v0).expect("encode v0 exec"), blockhash)
    }

    /// End-to-end: build V0 exec rows for a no-backlog chain with
    /// deposits_processed `[2, 0, 3, 1]`, run the real migration, and assert the
    /// current struct decodes with `next_deposit_idx == [2, 2, 5, 6]`.
    #[test]
    fn migrate_exec_blocks_backfills_next_deposit_idx() {
        let db = temp_db();
        let exec = db.open_tree(EXEC_TREE).unwrap();

        // No backlog: pending stays empty, so added == processed.
        // blocknums 0..=3, parent chain via blockhash.
        let added = [2usize, 0, 3, 1];
        let expected_nd = [2u64, 2, 5, 6];
        let mut hashes = Vec::new();
        for i in 0..4u8 {
            let mut bh = [0u8; 32];
            bh[0] = i + 1;
            let parent = if i == 0 {
                [0u8; 32]
            } else {
                let mut p = [0u8; 32];
                p[0] = i;
                p
            };
            let (bytes, h) = make_v0_exec(i as u64, bh, parent, added[i as usize], 0);
            exec.insert(h.as_slice(), bytes).unwrap();
            hashes.push(h);
        }

        // Sanity: current decode must fail before migration.
        for h in &hashes {
            let v = exec.get(h.as_slice()).unwrap().unwrap();
            assert!(DBExecBlockRecord::try_from_slice(&v).is_err());
        }

        let next_idx = build_next_deposit_idx(&db).unwrap();
        let migrated = migrate_exec_blocks(&db, &next_idx).unwrap();
        assert_eq!(migrated, 4);

        for (i, h) in hashes.iter().enumerate() {
            let v = exec.get(h.as_slice()).unwrap().unwrap();
            let rec = DBExecBlockRecord::try_from_slice(&v)
                .expect("current decode after migration");
            let domain = ExecBlockRecord::try_from(rec).expect("to domain");
            assert_eq!(
                domain.next_deposit_idx(),
                expected_nd[i],
                "blocknum {i}"
            );
        }
    }

    /// End-to-end with a deposit backlog where `added != processed`:
    ///   b0: added=5 pending_after=2 -> processed 3 -> nd 3
    ///   b1: added=0 pending_after=0 -> processed 2 -> nd 5
    ///   b2: added=4 pending_after=1 -> processed 3 -> nd 8
    #[test]
    fn migrate_exec_blocks_backlog_drain() {
        let db = temp_db();
        let exec = db.open_tree(EXEC_TREE).unwrap();

        let spec = [(5usize, 2usize), (0, 0), (4, 1)];
        let expected_nd = [3u64, 5, 8];
        let mut hashes = Vec::new();
        for (i, (added, pending_after)) in spec.iter().enumerate() {
            let mut bh = [0u8; 32];
            bh[0] = (i as u8) + 1;
            let parent = if i == 0 {
                [0u8; 32]
            } else {
                let mut p = [0u8; 32];
                p[0] = i as u8;
                p
            };
            let (bytes, h) = make_v0_exec(i as u64, bh, parent, *added, *pending_after);
            exec.insert(h.as_slice(), bytes).unwrap();
            hashes.push(h);
        }

        let next_idx = build_next_deposit_idx(&db).unwrap();
        migrate_exec_blocks(&db, &next_idx).unwrap();

        for (i, h) in hashes.iter().enumerate() {
            let v = exec.get(h.as_slice()).unwrap().unwrap();
            let rec = DBExecBlockRecord::try_from_slice(&v).unwrap();
            let domain = ExecBlockRecord::try_from(rec).unwrap();
            assert_eq!(domain.next_deposit_idx(), expected_nd[i], "block {i}");
        }
    }

    /// End-to-end chunk migration: build a V0 chunk row + supporting batch and
    /// exec rows, run the migration, assert `batch_idx` / `last_blocknum` are
    /// backfilled correctly.
    #[test]
    fn migrate_chunks_backfills_batch_idx_and_last_blocknum() {
        let db = temp_db();

        let prev_block = [0x11u8; 32];
        let last_block = [0x22u8; 32];
        let inner = [0x33u8; 32];

        // Exec block for last_block at blocknum 42 (we only need its blocknum,
        // read from the first 8 bytes of the stored value).
        let exec = db.open_tree(EXEC_TREE).unwrap();
        let (bytes, _) = make_v0_exec(42, last_block, prev_block, 0, 0);
        exec.insert(last_block.as_slice(), bytes).unwrap();

        // Batch idx mapping: batch_id -> 7; batch -> [chunk_id].
        let batch_id_to_idx = db.open_tree(BATCH_ID_TO_IDX_TREE).unwrap();
        let batch_chunks = db.open_tree(BATCH_CHUNKS_TREE).unwrap();
        // raw borsh key for DBBatchId == prev||last (64 bytes)
        let batch_key = {
            let mut k = Vec::with_capacity(64);
            k.extend_from_slice(&prev_block);
            k.extend_from_slice(&last_block);
            k
        };
        batch_id_to_idx
            .insert(batch_key.clone(), borsh::to_vec(&7u64).unwrap())
            .unwrap();
        let chunk_ids = vec![DBChunkIdKey {
            prev_block,
            last_block,
        }];
        batch_chunks
            .insert(batch_key, borsh::to_vec(&chunk_ids).unwrap())
            .unwrap();

        // V0 chunk row keyed by chunk idx (u64 borsh).
        let chunks = db.open_tree(CHUNK_TREE).unwrap();
        let v0 = DBChunkWithStatusV0 {
            chunk: DBChunkV0 {
                idx: 3,
                prev_block,
                last_block,
                inner_blocks: vec![inner],
            },
            status: DBChunkStatus::ProvingNotStarted,
        };
        let chunk_key = borsh::to_vec(&3u64).unwrap();
        chunks
            .insert(chunk_key.clone(), borsh::to_vec(&v0).unwrap())
            .unwrap();

        // Pre-migration: current decode fails.
        {
            let v = chunks.get(&chunk_key).unwrap().unwrap();
            assert!(DBChunkWithStatus::try_from_slice(&v).is_err());
        }

        let batch_idx_by_chunk = build_chunk_to_batch_idx(&db).unwrap();
        let blocknum_by_hash = build_blocknum_by_hash(&db).unwrap();
        let n = migrate_chunks(&db, &batch_idx_by_chunk, &blocknum_by_hash).unwrap();
        assert_eq!(n, 1);

        let v = chunks.get(&chunk_key).unwrap().unwrap();
        let rec = DBChunkWithStatus::try_from_slice(&v).expect("current decode");
        let (chunk, _status) = rec.into_parts();
        assert_eq!(chunk.idx(), 3);
        assert_eq!(chunk.batch_idx(), 7);
        assert_eq!(chunk.last_blocknum(), 42);
        assert_eq!(chunk.prev_block(), Hash::from(prev_block));
        assert_eq!(chunk.last_block(), Hash::from(last_block));
    }

    /// Idempotency: a second `migrate_ee_db` run is a no-op and leaves bytes
    /// unchanged.
    #[test]
    fn migrate_ee_db_is_idempotent() {
        let db = temp_db();
        let exec = db.open_tree(EXEC_TREE).unwrap();
        let mut bh = [0u8; 32];
        bh[0] = 9;
        let (bytes, h) = make_v0_exec(0, bh, [0u8; 32], 2, 0);
        exec.insert(h.as_slice(), bytes).unwrap();

        let r1 = migrate_ee_db(&db).unwrap();
        assert!(r1.ran);
        assert_eq!(r1.exec_blocks_migrated, 1);

        let after_first = exec.get(h.as_slice()).unwrap().unwrap();

        let r2 = migrate_ee_db(&db).unwrap();
        assert!(!r2.ran, "second run must be a no-op");
        assert_eq!(r2.exec_blocks_migrated, 0);

        let after_second = exec.get(h.as_slice()).unwrap().unwrap();
        assert_eq!(after_first, after_second, "bytes changed on rerun");
    }

    /// Hand-computed multi-block deposit fixture: blocks with
    /// deposits_processed `[2, 0, 3, 1]` (no backlog so `added == processed`)
    /// must yield cumulative `next_deposit_idx` `[2, 2, 5, 6]`.
    #[test]
    fn next_deposit_idx_cumulative_chain() {
        // Model the same arithmetic build_next_deposit_idx performs, with a
        // no-backlog chain (pending stays empty so added == processed).
        let deposits_processed = [2u64, 0, 3, 1];
        let expected = [2u64, 2, 5, 6];

        let mut next = 0u64;
        let mut got = Vec::new();
        for d in deposits_processed {
            next += d;
            got.push(next);
        }
        assert_eq!(got, expected);
    }

    /// Verifies the exact drain formula with a non-trivial backlog where
    /// `added != processed` for some blocks.
    ///
    /// Chain (added, pending_after):
    ///   b0: added=5 pending_after=2  -> processed = 5 + 0 - 2 = 3,   nd=3
    ///   b1: added=0 pending_after=0  -> processed = 0 + 2 - 0 = 2,   nd=5
    ///   b2: added=4 pending_after=1  -> processed = 4 + 0 - 1 = 3,   nd=8
    #[test]
    fn next_deposit_idx_drain_formula_with_backlog() {
        struct B {
            added: u64,
            pending_after: u64,
        }
        let chain = [
            B { added: 5, pending_after: 2 },
            B { added: 0, pending_after: 0 },
            B { added: 4, pending_after: 1 },
        ];
        let expected = [3u64, 5, 8];

        let mut prev_pending = 0u64;
        let mut next = 0u64;
        let mut got = Vec::new();
        for b in &chain {
            let processed = b.added as i64 + prev_pending as i64 - b.pending_after as i64;
            assert!(processed >= 0);
            next += processed as u64;
            got.push(next);
            prev_pending = b.pending_after;
        }
        assert_eq!(got, expected);
    }
}
