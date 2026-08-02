use std::time::Instant;

use alpen_ee_common::{Chunk, ChunkStatus};
use tracing::{debug, info};
use typed_sled::transaction::SledTransactional;

use super::super::super::EeNodeDBSled;
use crate::{serialization_types::DBChunkId, DbResult};

/// Bounds memory and transaction size while rebuilding derived work indexes.
const PAGE_SIZE: usize = 256;

/// Marker key 0 belonged to the pre-review schema names. The work-index schema rename changes the
/// typed-Sled tree names, so key 1 deliberately forces one rebuild for databases opened by those
/// development builds.
const CHUNK_WORK_INDEXES_V1_MARKER: u8 = 1;
const LEGACY_CHUNK_WORK_INDEXES_MARKER: u8 = 0;

#[derive(Debug, Default, PartialEq, Eq)]
struct RebuildStats {
    scanned_chunk_rows: usize,
    sealed_index_entries: usize,
    proof_pending_index_entries: usize,
    cleared_pages: usize,
    rebuilt_pages: usize,
}

pub(in crate::sleddb::maintenance) fn run_if_needed(db: &EeNodeDBSled) -> DbResult<()> {
    if db
        .ee_db_maintenance_tree
        .get(&CHUNK_WORK_INDEXES_V1_MARKER)?
        .unwrap_or(false)
    {
        debug!("EE chunk work-index maintenance already complete");
        return Ok(());
    }

    let started_at = Instant::now();
    info!(page_size = PAGE_SIZE, "rebuilding EE chunk work indexes");

    let stats = rebuild(db)?;

    // The marker is written last. If any earlier page fails or the process exits, the next startup
    // clears partial target rows and rebuilds from the authoritative chunk rows again.
    (&db.ee_db_maintenance_tree,).transaction_with_retry(
        db.config.backoff.as_ref(),
        db.config.retry_count.into(),
        |(maintenance_tree,)| {
            maintenance_tree.remove(&LEGACY_CHUNK_WORK_INDEXES_MARKER)?;
            maintenance_tree.insert(&CHUNK_WORK_INDEXES_V1_MARKER, &true)?;
            Ok(())
        },
    )?;

    info!(
        scanned_chunk_rows = stats.scanned_chunk_rows,
        sealed_index_entries = stats.sealed_index_entries,
        proof_pending_index_entries = stats.proof_pending_index_entries,
        cleared_pages = stats.cleared_pages,
        rebuilt_pages = stats.rebuilt_pages,
        elapsed_ms = started_at.elapsed().as_millis(),
        "rebuilt EE chunk work indexes"
    );

    Ok(())
}

fn rebuild(db: &EeNodeDBSled) -> DbResult<RebuildStats> {
    let mut stats = RebuildStats::default();

    loop {
        let keys = db
            .sealed_chunk_work_by_idx_tree
            .iter()
            .take(PAGE_SIZE)
            .map(|item| item.map(|(idx, _)| idx))
            .collect::<Result<Vec<_>, _>>()?;
        if keys.is_empty() {
            break;
        }

        (&db.sealed_chunk_work_by_idx_tree,).transaction_with_retry(
            db.config.backoff.as_ref(),
            db.config.retry_count.into(),
            |(sealed_tree,)| {
                for idx in &keys {
                    sealed_tree.remove(idx)?;
                }
                Ok(())
            },
        )?;
        stats.cleared_pages += 1;
    }

    loop {
        let keys = db
            .proof_pending_chunk_work_by_idx_tree
            .iter()
            .take(PAGE_SIZE)
            .map(|item| item.map(|(idx, _)| idx))
            .collect::<Result<Vec<_>, _>>()?;
        if keys.is_empty() {
            break;
        }

        (&db.proof_pending_chunk_work_by_idx_tree,).transaction_with_retry(
            db.config.backoff.as_ref(),
            db.config.retry_count.into(),
            |(pending_tree,)| {
                for idx in &keys {
                    pending_tree.remove(idx)?;
                }
                Ok(())
            },
        )?;
        stats.cleared_pages += 1;
    }

    let mut start_idx = Some(0u64);
    while let Some(page_start) = start_idx {
        let rows = db
            .chunk_by_idx_tree
            .range(page_start..)?
            .take(PAGE_SIZE)
            .collect::<Result<Vec<_>, _>>()?;
        let Some((last_idx, _)) = rows.last() else {
            break;
        };

        let mut sealed_work = Vec::new();
        let mut proof_pending_work = Vec::new();
        for (idx, db_chunk) in &rows {
            let (chunk, status): (Chunk, ChunkStatus) = db_chunk.clone().into_parts();
            let chunk_id = DBChunkId::from(chunk.id());
            match status {
                ChunkStatus::Sealed => sealed_work.push((*idx, chunk_id)),
                ChunkStatus::ProofPending(_) => proof_pending_work.push((*idx, chunk_id)),
                ChunkStatus::ProofReady(_) => {}
            }
        }

        (
            &db.sealed_chunk_work_by_idx_tree,
            &db.proof_pending_chunk_work_by_idx_tree,
        )
            .transaction_with_retry(
                db.config.backoff.as_ref(),
                db.config.retry_count.into(),
                |(sealed_tree, pending_tree)| {
                    for (idx, chunk_id) in &sealed_work {
                        sealed_tree.insert(idx, chunk_id)?;
                    }
                    for (idx, chunk_id) in &proof_pending_work {
                        pending_tree.insert(idx, chunk_id)?;
                    }
                    Ok(())
                },
            )?;

        stats.scanned_chunk_rows += rows.len();
        stats.sealed_index_entries += sealed_work.len();
        stats.proof_pending_index_entries += proof_pending_work.len();
        stats.rebuilt_pages += 1;
        start_idx = last_idx.checked_add(1);
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alpen_ee_common::{Chunk, ChunkStatus};
    use strata_acct_types::Hash;
    use strata_db_store_sled::SledDbConfig;
    use typed_sled::{SledDb, SledTree};

    use super::*;
    use crate::{
        serialization_types::{DBChunkId, DBChunkWithStatus},
        sleddb::{
            ChunkByIdxSchema, ChunkIdToIdxSchema, ProofPendingChunkWorkByIdxSchema,
            SealedChunkWorkByIdxSchema,
        },
    };

    fn hash_from_u64(value: u64) -> Hash {
        let mut bytes = [0u8; 32];
        bytes[24..].copy_from_slice(&value.to_be_bytes());
        Hash::from(bytes)
    }

    fn test_chunk(idx: u64) -> Chunk {
        Chunk::new(
            idx,
            hash_from_u64(idx),
            hash_from_u64(idx + 1),
            idx,
            0,
            Vec::new(),
        )
    }

    fn seed_chunk(sled_db: &SledDb, chunk: Chunk, status: ChunkStatus) {
        let idx = chunk.idx();
        let chunk_id = DBChunkId::from(chunk.id());
        let db_chunk = DBChunkWithStatus::new(chunk, status);
        let chunk_tree: SledTree<ChunkByIdxSchema> = sled_db.get_tree().unwrap();
        let chunk_id_tree: SledTree<ChunkIdToIdxSchema> = sled_db.get_tree().unwrap();

        chunk_tree.insert(&idx, &db_chunk).unwrap();
        chunk_id_tree.insert(&chunk_id, &idx).unwrap();
    }

    fn setup() -> (Arc<SledDb>, EeNodeDBSled) {
        let db = sled::Config::new().temporary(true).open().unwrap();
        let sled_db = Arc::new(SledDb::new(db).unwrap());
        let config = SledDbConfig::new_with_constant_backoff(2, 0);
        let ee_db = EeNodeDBSled::new(sled_db.clone(), config).unwrap();
        (sled_db, ee_db)
    }

    #[test]
    fn repairs_partial_indexes_and_replaces_legacy_marker() {
        let (sled_db, ee_db) = setup();
        let sealed_chunk = test_chunk(0);
        let pending_chunk = test_chunk(1);
        let stale_chunk_id = DBChunkId::from(test_chunk(99).id());
        seed_chunk(&sled_db, sealed_chunk, ChunkStatus::Sealed);
        seed_chunk(
            &sled_db,
            pending_chunk,
            ChunkStatus::ProofPending("task".to_string()),
        );
        seed_chunk(
            &sled_db,
            test_chunk(2),
            ChunkStatus::ProofReady(hash_from_u64(2)),
        );

        ee_db
            .sealed_chunk_work_by_idx_tree
            .insert(&99, &stale_chunk_id)
            .unwrap();
        ee_db
            .ee_db_maintenance_tree
            .insert(&LEGACY_CHUNK_WORK_INDEXES_MARKER, &true)
            .unwrap();

        run_if_needed(&ee_db).unwrap();

        let sealed_keys = ee_db
            .sealed_chunk_work_by_idx_tree
            .iter()
            .map(|item| item.unwrap().0)
            .collect::<Vec<_>>();
        let pending_keys = ee_db
            .proof_pending_chunk_work_by_idx_tree
            .iter()
            .map(|item| item.unwrap().0)
            .collect::<Vec<_>>();
        assert_eq!(sealed_keys, vec![0]);
        assert_eq!(pending_keys, vec![1]);
        assert_eq!(
            ee_db
                .ee_db_maintenance_tree
                .get(&LEGACY_CHUNK_WORK_INDEXES_MARKER)
                .unwrap(),
            None
        );
        assert_eq!(
            ee_db
                .ee_db_maintenance_tree
                .get(&CHUNK_WORK_INDEXES_V1_MARKER)
                .unwrap(),
            Some(true)
        );
    }

    #[test]
    fn completion_marker_makes_subsequent_run_a_noop() {
        let (sled_db, ee_db) = setup();
        seed_chunk(&sled_db, test_chunk(0), ChunkStatus::Sealed);
        run_if_needed(&ee_db).unwrap();

        seed_chunk(&sled_db, test_chunk(1), ChunkStatus::Sealed);
        run_if_needed(&ee_db).unwrap();

        let sealed_keys = ee_db
            .sealed_chunk_work_by_idx_tree
            .iter()
            .map(|item| item.unwrap().0)
            .collect::<Vec<_>>();
        assert_eq!(sealed_keys, vec![0]);
    }

    #[test]
    fn rebuild_is_bounded_and_restart_converges() {
        let (sled_db, ee_db) = setup();
        for idx in 0..(PAGE_SIZE as u64 * 2 + 1) {
            let status = if idx % 2 == 0 {
                ChunkStatus::Sealed
            } else {
                ChunkStatus::ProofPending(format!("task-{idx}"))
            };
            seed_chunk(&sled_db, test_chunk(idx), status);
        }

        let stale_chunk_id = DBChunkId::from(test_chunk(999).id());
        let sealed_tree: SledTree<SealedChunkWorkByIdxSchema> = sled_db.get_tree().unwrap();
        let pending_tree: SledTree<ProofPendingChunkWorkByIdxSchema> = sled_db.get_tree().unwrap();
        for idx in 1000..(1000 + PAGE_SIZE as u64 + 1) {
            sealed_tree.insert(&idx, &stale_chunk_id).unwrap();
            pending_tree.insert(&idx, &stale_chunk_id).unwrap();
        }

        let first = rebuild(&ee_db).unwrap();
        assert_eq!(first.rebuilt_pages, 3);
        assert_eq!(first.cleared_pages, 4);

        // No marker simulates a process exit before completion was claimed. Rebuilding again must
        // clear the partial result and converge to the same authoritative indexes.
        let second = rebuild(&ee_db).unwrap();
        assert_eq!(second.rebuilt_pages, 3);
        assert_eq!(second.cleared_pages, 3);
        assert_eq!(
            ee_db.sealed_chunk_work_by_idx_tree.iter().count(),
            PAGE_SIZE + 1
        );
        assert_eq!(
            ee_db.proof_pending_chunk_work_by_idx_tree.iter().count(),
            PAGE_SIZE
        );
    }
}
