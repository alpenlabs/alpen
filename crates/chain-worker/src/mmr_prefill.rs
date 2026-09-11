//! Shared helpers for DB-side OL MMR index prefill.

use strata_db_types::{DbError, DbResult};
use strata_ol_state_types_v1::MMR_SENTINEL_DUMMY_LEAF_HASH;
use strata_storage::{MmrId, MmrIndexManager};
use tokio::task::spawn_blocking;

/// Seeds the DB-side L1 block refs MMR index sentinel range.
///
/// Batches sentinel leaves for indices `0..=genesis_l1_height`, matching the
/// in-state L1 block refs MMR genesis prefill so DB leaf index equals L1 height.
/// The operation is idempotent when the index already has the expected prefix.
///
/// The caller must run this only while no other writer can append to
/// `L1BlockRefs`. A concurrent append violates the startup-only single-writer
/// contract and returns a precondition error. An incomplete prefix must already
/// contain only sentinel leaves; a peak mismatch returns an error before writing.
pub async fn prefill_l1_block_refs_mmr(
    mmr_index_mgr: &MmrIndexManager,
    genesis_l1_height: u64,
) -> DbResult<()> {
    let mmr_index_mgr = mmr_index_mgr.clone();
    spawn_blocking(move || prefill_l1_block_refs_mmr_blocking(&mmr_index_mgr, genesis_l1_height))
        .await
        .map_err(DbError::from)?
}

/// Blocking variant of [`prefill_l1_block_refs_mmr`].
///
/// The same startup-only single-writer contract applies.
pub fn prefill_l1_block_refs_mmr_blocking(
    mmr_index_mgr: &MmrIndexManager,
    genesis_l1_height: u64,
) -> DbResult<()> {
    let handle = mmr_index_mgr.get_handle(MmrId::L1BlockRefs);
    handle.prefill_repeated_leaves_blocking(MMR_SENTINEL_DUMMY_LEAF_HASH, genesis_l1_height + 1)
}
