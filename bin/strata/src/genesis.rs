//! OL genesis initialization for the new strata binary.

// TODO: Move this to a "node" crate when it's ready.

use std::{thread::sleep, time::Duration};

use anyhow::{Result, anyhow};
use strata_db_types::traits::BlockStatus;
use strata_identifiers::{Buf64, OLBlockCommitment};
use strata_ledger_types::AsmManifest;
use strata_ol_chain_types_new::{OLBlock, SignedOLBlockHeader};
use strata_ol_state_types::OLState;
use strata_ol_stf::{BlockComponents, BlockContext, BlockInfo, execute_and_complete_block};
use strata_params::Params;
use strata_primitives::{Buf32, L1BlockId};
use strata_storage::NodeStorage;
use tracing::{info, instrument};

/// Initialize the OL genesis block and state for a fresh database.
#[instrument(skip_all, fields(component = "ol_genesis"))]
pub(crate) fn init_ol_genesis(params: &Params, storage: &NodeStorage) -> Result<OLBlockCommitment> {
    info!("initializing OL genesis block and state");

    // Create initial OL state (uses genesis defaults)
    // TODO: initialize with a Snark EE account for Alpen. Possibly with rollup params.
    let mut ol_state = OLState::new_genesis();

    // Create genesis block info
    let genesis_l1 = &params.rollup().genesis_l1_view;
    let genesis_ts = genesis_l1.last_11_timestamps[10] as u64;
    let genesis_info = BlockInfo::new_genesis(genesis_ts);

    // Wait for ASM manifest for genesis to be available
    //
    // TODO: do this when btcio service is ready because otherwise asm won't get inputs(l1 block
    // commitments) to process
    //
    // let genesis_manifest =
    //     wait_for_genesis_manifest(storage, &genesis_l1.blkid(), MAX_ATTEMPTS, WAIT_INTERVAL_MS)?;

    let genesis_manifest = AsmManifest::new(
        genesis_l1.height_u64(),
        genesis_l1.blkid(),
        // TODO: Properly fetch manifest from db and populate this, btc reader should read L1 and
        // send events/msgs to asm worker for this to be correctly done
        Buf32::zero().into(),
        vec![],
    );

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

/// Wait for the genesis block manifest to be available in the database.
/// Retries periodically until the manifest is found or max attempts is reached.
#[expect(
    unused,
    reason = "will be used after btc reader and asm read blocks from l1"
)]
fn wait_for_genesis_manifest(
    storage: &NodeStorage,
    block_id: &L1BlockId,
    max_attempts: u32,
    wait_interval_ms: u64,
) -> Result<AsmManifest> {
    for attempt in 0..max_attempts {
        if let Some(manifest) = storage.l1().get_block_manifest(block_id)? {
            if attempt > 0 {
                info!("Block manifest found after {} attempts", attempt);
            }
            return Ok(manifest);
        }

        if attempt == 0 {
            info!(
                "Waiting for block manifest {} to be available in database...",
                block_id
            );
        }
        info!(
            "Still waiting for block manifest {} (attempt {}/{})",
            block_id,
            attempt + 1,
            max_attempts
        );

        sleep(Duration::from_millis(wait_interval_ms));
    }

    Err(anyhow!(
        "Block manifest {} not found after {} seconds",
        block_id,
        max_attempts
    ))
}
