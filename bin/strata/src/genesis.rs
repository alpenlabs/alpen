//! OL genesis initialization for the new strata binary.

// TODO: Move this to a "node" crate when it's ready.

use anyhow::{Result, anyhow};
use strata_db_types::traits::BlockStatus;
use strata_identifiers::{Buf64, OLBlockCommitment};
use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
use strata_ol_state_types::OLState;
use strata_ol_stf::{BlockComponents, BlockContext, BlockInfo, execute_and_complete_block};
use strata_params::Params;
use strata_storage::NodeStorage;
use tracing::{debug, info, instrument};

/// Initialize the OL genesis block and state for a fresh database.
#[instrument(skip_all, fields(component = "ol_genesis"))]
pub(crate) fn init_ol_genesis(params: &Params, storage: &NodeStorage) -> Result<OLBlockCommitment> {
    debug!("initializing OL genesis block and state");

    // Create initial OL state (uses genesis defaults)
    let mut ol_state = OLState::new_genesis();

    // Create genesis block info
    let genesis_l1 = &params.rollup().genesis_l1_view;
    let genesis_ts = genesis_l1.last_11_timestamps[10] as u64;
    let genesis_info = BlockInfo::new_genesis(genesis_ts);

    // Create empty ASM manifest for genesis
    let genesis_manifest = storage
        .l1()
        .get_block_manifest(&genesis_l1.blkid())?
        .ok_or_else(|| anyhow!("Can't find genesis l1 manisfest"))?;

    // Build genesis block components
    let genesis_components = BlockComponents::new_manifests(vec![genesis_manifest]);

    // Execute genesis block through the OL STF
    let block_context = BlockContext::new(&genesis_info, None);
    let genesis_block =
        execute_and_complete_block(&mut ol_state, block_context, genesis_components)?;

    // Create signed header (genesis uses zero signature)
    let signed_header = SignedOLBlockHeader::new(genesis_block.header().clone(), Buf64::zero());
    let ol_block = OLBlock::new(signed_header, genesis_block.body().clone());
    let genesis_blkid = genesis_block.header().compute_blkid();

    storage.ol_block().put_block_data_blocking(ol_block)?;
    storage
        .ol_block()
        .set_block_status_blocking(genesis_blkid, BlockStatus::Valid)?;

    // Store genesis OL state
    let commitment = OLBlockCommitment::new(0, genesis_blkid);
    storage
        .ol_state()
        .put_toplevel_ol_state_blocking(commitment, ol_state)?;

    info!(%genesis_blkid, slot = 0, "OL genesis initialization complete");
    Ok(commitment)
}
