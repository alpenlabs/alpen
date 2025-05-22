// TODO this all needs to be reworked to just follow what the FCM state
// publishing is, waiting for that to be ready before getting started

use anyhow::{Context, Result};
use revm_primitives::B256;
use strata_db::errors::DbError;
use strata_primitives::params::RollupParams;
use strata_state::{block::L2BlockBundle, chain_state::ChainstateEntry, id::L2BlockId};
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
        None => Ok(evm_genesis_block_hash(rollup_params)),
    }
}

pub fn evm_genesis_block_hash(rollup_params: &RollupParams) -> revm_primitives::FixedBytes<32> {
    info!("preparing EVM initial state from genesis");
    let evm_genesis_block_hash =
        revm_primitives::FixedBytes(*rollup_params.evm_genesis_block_hash.as_ref());
    evm_genesis_block_hash
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
    let chsman = storage.chainstate();

    let last_write_idx = match chsman.get_last_write_idx_blocking() {
        Ok(idx) => idx,
        Err(DbError::NotBootstrapped) => {
            // before genesis block ready; use hardcoded genesis state
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };

    Ok(chsman.get_toplevel_chainstate_blocking(last_write_idx)?)
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
