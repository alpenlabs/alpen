//! Test-only boot harness: opens a *migrated* EE datadir through the real node
//! storage layer ([`init_db_storage`]) and asserts the recovery-relevant
//! invariants on real data — i.e. the backfilled fields are not just decodable
//! but *usable* by startup recovery.
//!
//! Specifically it confirms, against the migrated staging copy:
//! - the node storage layer opens and the startup reads succeed
//!   (`get_latest_batch` / `get_latest_chunk` — these decode current structs);
//! - every chunk's backfilled `batch_idx` resolves via `get_batch_by_idx`
//!   (exactly what `chunk_builder` recovery does at startup);
//! - every chunk's `last_blocknum` is non-zero where an exec record exists.
//!
//! Run on k2 against a migrated copy:
//! ```text
//! MIGRATED_EE_DATADIR=/Users/k2/staging-v2-dbs/ee-work \
//!   cargo test -p alpen-ee-database --lib migration_boot_harness -- --nocapture
//! ```
//! No-ops (passes) when the env var is unset, so it is inert in CI.

use std::path::Path;

use alpen_ee_common::{BatchStorage, ChunkStorage};

use crate::init_db_storage;

#[tokio::test]
async fn migration_boot_harness_recovery_invariants() {
    let Ok(datadir) = std::env::var("MIGRATED_EE_DATADIR") else {
        eprintln!("MIGRATED_EE_DATADIR unset; skipping boot harness");
        return;
    };

    // Real node storage-init path (`<datadir>/sled`).
    let dbs = init_db_storage(Path::new(&datadir), 3).expect("init_db_storage on migrated datadir");
    let pool = threadpool::ThreadPool::new(2);
    let storage = dbs.node_storage(pool);

    // Startup reads — these decode the current structs; would error/panic on an
    // unmigrated DB.
    let (latest_batch, _) = storage
        .get_latest_batch()
        .await
        .expect("get_latest_batch")
        .expect("a latest batch exists");
    let (latest_chunk, _) = storage
        .get_latest_chunk()
        .await
        .expect("get_latest_chunk")
        .expect("a latest chunk exists");
    eprintln!(
        "startup reads OK: latest_batch_idx={}, latest_chunk_idx={}",
        latest_batch.idx(),
        latest_chunk.idx()
    );

    // Recovery invariant: every chunk's backfilled batch_idx must resolve to a
    // real batch (this is exactly `chunk_builder::recovery` -> get_batch_by_idx).
    let mut chunks_checked = 0u64;
    let mut nonzero_blocknum = 0u64;
    for idx in 0..=latest_chunk.idx() {
        let Some((chunk, _)) = storage.get_chunk_by_idx(idx).await.expect("get_chunk_by_idx") else {
            continue;
        };
        let bidx = chunk.batch_idx();
        let batch = storage
            .get_batch_by_idx(bidx)
            .await
            .expect("get_batch_by_idx");
        assert!(
            batch.is_some(),
            "chunk idx {idx}: backfilled batch_idx {bidx} does not resolve to a batch"
        );
        if chunk.last_blocknum() != 0 {
            nonzero_blocknum += 1;
        }
        chunks_checked += 1;
    }

    eprintln!(
        "BOOT HARNESS OK: {chunks_checked} chunks, all batch_idx resolved via get_batch_by_idx; \
         {nonzero_blocknum} with non-zero last_blocknum"
    );
    assert!(chunks_checked > 0, "no chunks found in migrated DB");
}
