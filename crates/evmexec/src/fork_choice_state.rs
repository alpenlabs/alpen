// TODO this all needs to be reworked to just follow what the FCM state
// publishing is, waiting for that to be ready before getting started

use anyhow::{Context, Result};
use revm_primitives::B256;
use strata_db::errors::DbError;
use strata_ol_chain_types::{L2BlockBundle, L2BlockId};
use strata_ol_chainstate_types::ChainstateEntry;
use strata_params::RollupParams;
use strata_storage::*;
use tracing::*;

use crate::block::EVML2Block;

pub fn fetch_init_fork_choice_state(
    storage: &NodeStorage,
    rollup_params: &RollupParams,
) -> Result<B256> {
    // TODO switch these logs to debug
    match get_last_chainstate(storage)? {
        Some(chs) => {
            let slot = chs.state().chain_tip_slot();
            let tip = chs.tip_blockid();
            info!(%slot, %tip, "preparing EVM initial state from chainstate");
            compute_evm_fc_state_from_chainstate(tip, storage)
        }
        None => {
            info!("preparing EVM initial state from genesis");
            let evm_genesis_block_hash =
                revm_primitives::FixedBytes(*rollup_params.evm_genesis_block_hash.as_ref());
            Ok(evm_genesis_block_hash)
        }
    }
}

fn compute_evm_fc_state_from_chainstate(
    tip_blockid: &L2BlockId,
    storage: &NodeStorage,
) -> Result<B256> {
    let l2man = storage.l2();
    let latest_evm_block_hash =
        get_evm_block_hash_by_id(tip_blockid, l2man)?.expect("evmexec: missing expected block");
    Ok(latest_evm_block_hash)
}

fn get_last_chainstate(storage: &NodeStorage) -> Result<Option<ChainstateEntry>> {
    let tip_blkid = match storage.l2().get_tip_block_blocking() {
        Ok(id) => id,
        Err(DbError::NotBootstrapped) => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    Ok(storage
        .chainstate()
        .get_slot_write_batch_blocking(tip_blkid)?
        .map(|wb| ChainstateEntry::new(wb.into_toplevel(), tip_blkid)))
}

fn get_evm_block_hash_by_id(
    block_id: &L2BlockId,
    l2man: &L2BlockManager,
) -> anyhow::Result<Option<B256>> {
    l2man
        .get_block_data_blocking(block_id)?
        .map(|bundle| compute_evm_block_hash(&bundle))
        .transpose()
}

fn compute_evm_block_hash(l2_block: &L2BlockBundle) -> Result<B256> {
    EVML2Block::try_extract(l2_block)
        .map(|block| block.block_hash())
        .context("Failed to convert L2Block to EVML2Block")
}
