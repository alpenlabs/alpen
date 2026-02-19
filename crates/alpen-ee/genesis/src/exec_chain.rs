//! For handling deterministic genesis blocks used in EE.

use alpen_ee_common::ExecBlockStorage;
use alpen_ee_config::AlpenEeConfig;
use eyre::Context;
use strata_identifiers::OLBlockCommitment;
use tracing::info;

use crate::build_genesis_exec_block;

pub async fn ensure_finalized_exec_chain_genesis<TStorage: ExecBlockStorage>(
    config: &AlpenEeConfig,
    genesis_ol_block: OLBlockCommitment,
    storage: &TStorage,
) -> eyre::Result<()> {
    let genesis_ee_blockhash = config.params().genesis_blockhash().0.into();
    info!(%genesis_ee_blockhash, "genesis ee blockhash");
    let (genesis_block, genesis_block_payload) =
        build_genesis_exec_block(config.params(), genesis_ol_block);

    // If exists, does not overwrite
    storage
        .save_exec_block(genesis_block, genesis_block_payload)
        .await
        .map_err(eyre::Error::from)
        .context("ensure_finalized_exec_chain_genesis: failed to create genesis exec block")?;
    // Inserts if empty, checks genesis blockhash is correct if exists.
    storage
        .init_finalized_chain(genesis_ee_blockhash)
        .await
        .map_err(eyre::Error::from)
        .context("ensure_finalized_exec_chain_genesis: failed to set genesis exec block")?;

    Ok(())
}
