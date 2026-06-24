//! OL genesis initialization for the new strata binary.

use std::collections::BTreeMap;

use anyhow::Result;
use strata_db_types::{MmrId, ol_state_index::IndexingWrites, traits::BlockStatus};
use strata_ol_genesis::{GenesisArtifacts, build_genesis_artifacts};
use strata_ol_params::OLParams;
use strata_ol_state_types::MMR_SENTINEL_DUMMY_LEAF_HASH;
use strata_primitives::OLBlockCommitment;
use strata_storage::NodeStorage;
use tracing::{info, instrument};

/// Initialize the OL genesis block and state for a fresh database.
#[instrument(skip_all, fields(component = "ol_genesis"))]
pub(crate) fn init_ol_genesis(
    ol_params: &OLParams,
    storage: &NodeStorage,
) -> Result<OLBlockCommitment> {
    info!("initializing OL genesis block and state");

    let GenesisArtifacts {
        ol_state,
        ol_block,
        commitment,
        epoch_summary,
    } = build_genesis_artifacts(ol_params)?;
    let genesis_blkid = *commitment.blkid();

    prefill_l1_block_refs_mmr(ol_params, storage)?;

    // Seed epoch-0 indexing with all genesis accounts as created accounts.
    // Genesis epoch is finalized at boot, so its commitment is known here;
    // no per-account updates or inbox writes at genesis.
    let created_accounts = ol_state
        .ledger
        .accounts
        .iter()
        .map(|entry| {
            info!(%entry.id, "inserting account info");
            entry.id
        })
        .collect::<Vec<_>>();
    storage.ol_state_indexing().apply_epoch_indexing_blocking(
        epoch_summary.get_epoch_commitment(),
        IndexingWrites::new(created_accounts, BTreeMap::new(), BTreeMap::new()),
    )?;

    storage.ol_block().put_block_data_blocking(ol_block)?;
    storage
        .ol_block()
        .set_block_status_blocking(genesis_blkid, BlockStatus::Valid)?;

    // Store genesis OL state
    storage
        .ol_state()
        .put_toplevel_ol_state_blocking(commitment, ol_state)?;

    storage
        .ol_checkpoint()
        .insert_epoch_summary_blocking(epoch_summary)?;

    // Update the canonical block index for first block.
    storage
        .ol_block()
        .replace_canonical_suffix_from_blocking(0, vec![genesis_blkid])?;

    info!(%genesis_blkid, slot = 0, "OL genesis initialization complete");
    Ok(commitment)
}

fn prefill_l1_block_refs_mmr(ol_params: &OLParams, storage: &NodeStorage) -> Result<()> {
    let target_count = ol_params.last_l1_block.height() as u64 + 1;
    let handle = storage.mmr_index().get_handle(MmrId::L1BlockRefs);
    let current_count = handle.get_num_leaves_blocking()?;

    for _ in current_count..target_count {
        handle.append_leaf_blocking(MMR_SENTINEL_DUMMY_LEAF_HASH)?;
    }

    Ok(())
}
