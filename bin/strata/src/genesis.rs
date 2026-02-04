//! OL genesis initialization for the new strata binary.

use std::{thread::sleep, time::Duration};

use anyhow::{Result, anyhow};
use strata_db_types::traits::BlockStatus;
use strata_identifiers::OLBlockCommitment;
use strata_ledger_types::AsmManifest;
use strata_ol_genesis::{GenesisArtifacts, build_genesis_artifacts};
use strata_params::Params;
use strata_primitives::L1BlockId;
use strata_storage::NodeStorage;
use tracing::{info, instrument};

/// Initialize the OL genesis block and state for a fresh database.
#[instrument(skip_all, fields(component = "ol_genesis"))]
pub(crate) fn init_ol_genesis(params: &Params, storage: &NodeStorage) -> Result<OLBlockCommitment> {
    info!("initializing OL genesis block and state");

    // Wait for ASM manifest for genesis to be available.
    //
    // TODO: do this when btcio service is ready because otherwise asm won't get inputs(l1 block
    // commitments) to process.
    //
    // let genesis_manifest =
    //     wait_for_genesis_manifest(storage, &genesis_l1.blkid(), MAX_ATTEMPTS, WAIT_INTERVAL_MS)?;

    let GenesisArtifacts {
        ol_state,
        ol_block,
        commitment,
    } = build_genesis_artifacts(params)?;
    let genesis_blkid = *commitment.blkid();

    storage.ol_block().put_block_data_blocking(ol_block)?;
    storage
        .ol_block()
        .set_block_status_blocking(genesis_blkid, BlockStatus::Valid)?;

    // Store genesis OL state
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
