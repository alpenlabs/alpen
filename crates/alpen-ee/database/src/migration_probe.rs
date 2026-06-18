//! Test-only empirical probe for the post-#1849 EE sled Borsh layout breaks.
//!
//! Confirms, against a *real* staging EE sled DB, that (a) the current structs
//! fail to decode old rows (the #1851 `DBChunk` / #1947 `DBExecBlockRecord`
//! breaks are real) and (b) the V0 mirror structs decode them — i.e. our
//! migration's read side is correct. Run on k2 against the staging copy:
//!
//! ```text
//! STAGING_EE_SLED=/Users/k2/staging-v2-dbs/ee-sled/sled \
//!   cargo test -p alpen-ee-database --lib migration_probe -- --nocapture
//! ```
//!
//! No-ops (passes) when `STAGING_EE_SLED` is unset, so it is inert in CI.

use std::{collections::HashMap, env};

use alpen_ee_common::ExecBlockRecord;
use borsh::{BorshDeserialize, BorshSerialize};
use ssz::Decode;
use strata_ee_chain_types::ExecBlockPackage;

use crate::serialization_types::{DBChunkWithStatus, DBExecBlockRecord};

/// V0 (pre-#1851) chunk: no `last_blocknum` / `batch_idx`.
#[derive(BorshDeserialize, BorshSerialize)]
struct DBChunkV0 {
    idx: u64,
    prev_block: [u8; 32],
    last_block: [u8; 32],
    inner_blocks: Vec<[u8; 32]>,
}

/// Mirror of `DBChunkStatus` (unchanged by #1851), needed for the V0 wrapper.
#[derive(BorshDeserialize, BorshSerialize)]
enum DBChunkStatusMirror {
    ProvingNotStarted,
    ProofPending(String),
    ProofReady([u8; 32]),
}

/// V0 wrapper as stored in `ChunkByIdxSchema` before #1851.
#[derive(BorshDeserialize, BorshSerialize)]
struct DBChunkWithStatusV0 {
    chunk: DBChunkV0,
    status: DBChunkStatusMirror,
}

#[test]
fn migration_probe_ee_layout_breaks() {
    let Ok(path) = env::var("STAGING_EE_SLED") else {
        eprintln!("STAGING_EE_SLED unset; skipping probe");
        return;
    };
    let db = sled::open(&path).expect("open staging EE sled");
    let trees: Vec<String> = db
        .tree_names()
        .iter()
        .map(|t| String::from_utf8_lossy(t).into_owned())
        .collect();
    eprintln!("trees: {trees:?}");

    // --- ChunkByIdxSchema: new must fail, V0 must succeed ---
    let chunk_tree = db.open_tree("ChunkByIdxSchema").expect("open chunk tree");
    let (mut rows, mut new_ok, mut v0_ok) = (0u64, 0u64, 0u64);
    for kv in chunk_tree.iter() {
        let (_k, v) = kv.expect("chunk iter");
        rows += 1;
        if DBChunkWithStatus::try_from_slice(&v).is_ok() {
            new_ok += 1;
        }
        if DBChunkWithStatusV0::try_from_slice(&v).is_ok() {
            v0_ok += 1;
        }
    }
    eprintln!("CHUNK: rows={rows} new_decode_ok={new_ok} v0_decode_ok={v0_ok}");

    // --- ExecBlockSchema: new must fail on old rows ---
    let exec_tree = db.open_tree("ExecBlockSchema").expect("open exec tree");
    let (mut erows, mut enew_ok) = (0u64, 0u64);
    for kv in exec_tree.iter() {
        let (_k, v) = kv.expect("exec iter");
        erows += 1;
        if DBExecBlockRecord::try_from_slice(&v).is_ok() {
            enew_ok += 1;
        }
    }
    eprintln!("EXEC: rows={erows} new_decode_ok={enew_ok}");

    // Empirical expectations: the break is real (new fails) and the V0 read works.
    assert!(rows > 0, "no chunk rows in staging DB");
    assert_eq!(new_ok, 0, "current DBChunk unexpectedly decoded old rows");
    assert_eq!(v0_ok, rows, "V0 chunk layout failed to decode some old rows");
    assert!(erows > 0, "no exec-block rows in staging DB");
    assert_eq!(enew_ok, 0, "current DBExecBlockRecord unexpectedly decoded old rows");
}

/// Borsh mirror of `DBEeAccountState` (account_state.rs) for raw V0 decode.
#[derive(BorshDeserialize, BorshSerialize)]
struct DBSubjectIdMirror([u8; 32]);
#[derive(BorshDeserialize, BorshSerialize)]
struct DBBitcoinAmountMirror(u64);
#[derive(BorshDeserialize, BorshSerialize)]
struct DBSubjectDepositDataMirror {
    dest: DBSubjectIdMirror,
    value: DBBitcoinAmountMirror,
}
#[derive(BorshDeserialize, BorshSerialize)]
enum DBPendingInputEntryMirror {
    Deposit(DBSubjectDepositDataMirror),
}
#[derive(BorshDeserialize, BorshSerialize)]
struct DBPendingFinclEntryMirror {
    epoch: u32,
    raw_tx_hash: [u8; 32],
}
#[derive(BorshDeserialize, BorshSerialize)]
struct DBEeAccountStateMirror {
    last_exec_blkid: [u8; 32],
    last_exec_state_root: [u8; 32],
    pending_inputs: Vec<DBPendingInputEntryMirror>,
    pending_fincls: Vec<DBPendingFinclEntryMirror>,
}

#[derive(BorshDeserialize, BorshSerialize)]
struct DBMessageEntryMirror {
    source: [u8; 32],
    incl_epoch: u32,
    payload_value_sats: u64,
    payload_data: Vec<u8>,
}

/// V0 (pre-#1947) exec-block record: no `next_deposit_idx`.
#[derive(BorshDeserialize, BorshSerialize)]
struct DBExecBlockRecordV0 {
    blocknum: u64,
    parent_blockhash: [u8; 32],
    timestamp_ms: u64,
    // OLBlockCommitment is borsh-encoded as (slot: u64, blkid: [u8;32]) — but we
    // don't need its internals here; capture it via the real type below.
    ol_block: strata_identifiers::OLBlockCommitment,
    package_ssz: Vec<u8>,
    account_state: DBEeAccountStateMirror,
    next_inbox_msg_idx: u64,
    messages: Vec<DBMessageEntryMirror>,
}

/// Post-migration inverse check: after running the migration on a copy, EVERY
/// row in `ChunkByIdxSchema` and `ExecBlockSchema` must decode with the CURRENT
/// structs. Point `MIGRATED_EE_SLED` at the migrated copy.
#[test]
fn migration_probe_post_migration_decode() {
    let Ok(path) = env::var("MIGRATED_EE_SLED") else {
        eprintln!("MIGRATED_EE_SLED unset; skipping post-migration EE decode check");
        return;
    };
    let db = sled::open(&path).expect("open migrated EE sled");

    let chunk_tree = db.open_tree("ChunkByIdxSchema").expect("open chunk tree");
    let (mut crows, mut cok) = (0u64, 0u64);
    for kv in chunk_tree.iter() {
        let (_k, v) = kv.expect("chunk iter");
        crows += 1;
        if DBChunkWithStatus::try_from_slice(&v).is_ok() {
            cok += 1;
        }
    }

    let exec_tree = db.open_tree("ExecBlockSchema").expect("open exec tree");
    let (mut erows, mut eok) = (0u64, 0u64);
    let (mut nd_max, mut nd_nonzero) = (0u64, 0u64);
    for kv in exec_tree.iter() {
        let (_k, v) = kv.expect("exec iter");
        erows += 1;
        if let Ok(rec) = DBExecBlockRecord::try_from_slice(&v) {
            eok += 1;
            // Sample next_deposit_idx via the domain conversion.
            if let Ok(d) = ExecBlockRecord::try_from(rec) {
                let nd = d.next_deposit_idx();
                nd_max = nd_max.max(nd);
                if nd > 0 {
                    nd_nonzero += 1;
                }
            }
        }
    }
    eprintln!(
        "POST-MIGRATION EE: chunk {cok}/{crows} exec {eok}/{erows} next_deposit_idx(max={nd_max} nonzero_rows={nd_nonzero})"
    );
    assert_eq!(cok, crows, "some chunk rows fail current decode post-migration");
    assert_eq!(eok, erows, "some exec rows fail current decode post-migration");
}

/// Validates the two candidate `next_deposit_idx` reconstruction methods against
/// the real staging EE DB:
///   A) naive: cumulative sum of `package.inputs.subject_deposits().len()`.
///   B) exact: cumulative sum of `package.inputs.len() + parent.pending.len()
///      - block.pending.len()` (deposits actually drained from the queue).
/// Reports whether the two ever diverge (i.e. whether a deposit backlog/cap
/// ever occurred in staging), which decides whether the package-count shortcut
/// is safe.
#[test]
fn migration_probe_next_deposit_idx_methods() {
    let Ok(path) = env::var("STAGING_EE_SLED") else {
        eprintln!("STAGING_EE_SLED unset; skipping deposit-idx probe");
        return;
    };
    let db = sled::open(&path).expect("open staging EE sled");
    let exec_tree = db.open_tree("ExecBlockSchema").expect("open exec tree");

    struct BlockInfo {
        blocknum: u64,
        parent_blockhash: [u8; 32],
        pkg_inputs: u64,
        pending_len: u64,
    }
    // raw key bytes (borsh(Hash) == 32 raw bytes) -> info
    let mut by_blockhash: HashMap<[u8; 32], BlockInfo> = HashMap::new();
    let mut multi_per_blocknum: HashMap<u64, u64> = HashMap::new();
    let mut decode_fail = 0u64;

    for kv in exec_tree.iter() {
        let (k, v) = kv.expect("exec iter");
        let rec = match DBExecBlockRecordV0::try_from_slice(&v) {
            Ok(r) => r,
            Err(_) => {
                decode_fail += 1;
                continue;
            }
        };
        let pkg = ExecBlockPackage::from_ssz_bytes(&rec.package_ssz)
            .expect("decode package ssz from staging");
        let pkg_inputs = pkg.inputs().subject_deposits().len() as u64;
        // key for ExecBlockSchema is borsh(Hash) which == 32 raw bytes.
        let mut blockhash = [0u8; 32];
        assert_eq!(k.len(), 32, "exec key not 32 bytes: {}", k.len());
        blockhash.copy_from_slice(&k);
        *multi_per_blocknum.entry(rec.blocknum).or_insert(0) += 1;
        by_blockhash.insert(
            blockhash,
            BlockInfo {
                blocknum: rec.blocknum,
                parent_blockhash: rec.parent_blockhash,
                pkg_inputs,
                pending_len: rec.account_state.pending_inputs.len() as u64,
            },
        );
    }

    let dup: Vec<(u64, u64)> = multi_per_blocknum
        .iter()
        .filter(|(_, c)| **c > 1)
        .map(|(b, c)| (*b, *c))
        .collect();
    eprintln!(
        "DEPOSIT-PROBE: rows={} decode_fail={} distinct_blocknums={} blocknums_with_multiple_blocks={:?}",
        by_blockhash.len(),
        decode_fail,
        multi_per_blocknum.len(),
        dup
    );

    // Method A (naive) and Method B (exact) computed per-block, then compared.
    // Both need the parent's pending length; chain via parent_blockhash.
    let mut divergences = 0u64;
    let mut method_b_negative = 0u64;
    let zero_hash = [0u8; 32];
    for info in by_blockhash.values() {
        let method_a = info.pkg_inputs;
        // exact processed = added + parent_pending - block_pending
        let parent_pending = if info.parent_blockhash == zero_hash {
            0
        } else {
            match by_blockhash.get(&info.parent_blockhash) {
                Some(p) => p.pending_len,
                None => {
                    // parent not in stored set (e.g. genesis/pruned); skip exactness check
                    continue;
                }
            }
        };
        let added = info.pkg_inputs;
        let processed_signed = added as i64 + parent_pending as i64 - info.pending_len as i64;
        if processed_signed < 0 {
            method_b_negative += 1;
            continue;
        }
        let method_b = processed_signed as u64;
        if method_a != method_b {
            divergences += 1;
            if divergences <= 10 {
                eprintln!(
                    "  DIVERGE blocknum={} method_a(pkg_inputs)={} method_b(drained)={} (parent_pending={} block_pending={})",
                    info.blocknum, method_a, method_b, parent_pending, info.pending_len
                );
            }
        }
    }
    eprintln!(
        "DEPOSIT-PROBE: method_a_vs_b divergences={} method_b_negative={}",
        divergences, method_b_negative
    );
}
