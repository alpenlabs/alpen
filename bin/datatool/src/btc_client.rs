//! Bitcoin client functionality for Strata `datatool` binary.
//!
//! This module contains Bitcoin RPC client operations and is feature-gated
//! behind the `btc-client` feature flag.

use bitcoin::{params::Params, CompactTarget};
use bitcoind_async_client::{traits::Reader, Auth, Client};
use strata_btc_types::BlockHashExt;
use strata_btc_verification::{get_relative_difficulty_adjustment_height, L1Anchor};
use strata_primitives::l1::{BtcParams, L1BlockCommitment, L1Height};

use crate::args::BitcoindConfig;

/// Fetches the genesis L1 anchor using the provided Bitcoin RPC configuration.
///
/// Creates a Bitcoin client from the config and fetches the [`L1Anchor`] at the
/// specified block height.
pub(crate) async fn fetch_l1_anchor_with_config(
    config: &BitcoindConfig,
    block_height: L1Height,
) -> anyhow::Result<L1Anchor> {
    let client = create_client(config)?;
    fetch_l1_anchor(&client, block_height).await
}

async fn fetch_l1_anchor(client: &impl Reader, block_height: L1Height) -> anyhow::Result<L1Anchor> {
    // Create BTC parameters based on the current network.
    let network = client.network().await?;
    let btc_params = BtcParams::from(Params::from(network));

    // Get the difficulty adjustment block just before the given block height,
    // representing the start of the current epoch.
    let current_epoch_start_height =
        get_relative_difficulty_adjustment_height(0, block_height, btc_params.inner());
    let current_epoch_start_header = client
        .get_block_header_at(current_epoch_start_height as u64)
        .await?;

    // Fetch the block header at the height.
    let block_header = client.get_block_header_at(block_height as u64).await?;

    // Compute the block ID for the verified block.
    let block_id = block_header.block_hash().to_l1_block_id();

    // If (block_height + 1) is the start of the new epoch, we need to calculate the
    // next_target, else next_target will be the current block's target.
    let next_target =
        if (block_height as u64 + 1).is_multiple_of(btc_params.difficulty_adjustment_interval()) {
            CompactTarget::from_next_work_required(
                block_header.bits,
                (block_header.time - current_epoch_start_header.time) as u64,
                &btc_params,
            )
            .to_consensus()
        } else {
            block_header.target().to_compact_lossy().to_consensus()
        };

    Ok(L1Anchor {
        block: L1BlockCommitment::new(block_height, block_id),
        next_target,
        epoch_start_timestamp: current_epoch_start_header.time,
        network,
    })
}

/// Creates a Bitcoin RPC client from the provided configuration.
fn create_client(config: &BitcoindConfig) -> anyhow::Result<Client> {
    let auth = Auth::UserPass(config.rpc_user.clone(), config.rpc_password.clone());
    Client::new(config.rpc_url.clone(), auth, None, None, None)
        .map_err(|e| anyhow::anyhow!("Failed to create Bitcoin RPC client: {}", e))
}
