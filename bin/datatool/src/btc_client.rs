//! Bitcoin client functionality for Strata `datatool` binary.
//!
//! This module contains Bitcoin RPC client operations and is feature-gated
//! behind the `btc-client` feature flag.

use bitcoin::CompactTarget;
use bitcoind_async_client::{traits::Reader, Client};
use strata_asm_types::get_relative_difficulty_adjustment_height;
use strata_primitives::{
    constants::TIMESTAMPS_FOR_MEDIAN,
    l1::{BtcParams, GenesisL1View, L1BlockCommitment, L1BlockId},
};

use crate::args::BitcoindConfig;

/// Fetches genesis L1 view using the provided Bitcoin RPC configuration.
///
/// Creates a Bitcoin client from the config and fetches the genesis L1 view
/// at the specified block height.
pub(crate) async fn fetch_genesis_l1_view_with_config(
    config: &BitcoindConfig,
    block_height: u64,
) -> anyhow::Result<GenesisL1View> {
    let client = create_client(config)?;
    fetch_genesis_l1_view(&client, block_height).await
}

async fn fetch_genesis_l1_view(
    client: &impl Reader,
    block_height: u64,
) -> anyhow::Result<GenesisL1View> {
    // Create BTC parameters based on the current network.
    let network = client.network().await?;
    let btc_params = BtcParams::from(bitcoin::params::Params::from(network));

    // Get the difficulty adjustment block just before the given block height,
    // representing the start of the current epoch.
    let current_epoch_start_height =
        get_relative_difficulty_adjustment_height(0, block_height, btc_params.inner());
    let current_epoch_start_header = client
        .get_block_header_at(current_epoch_start_height)
        .await?;

    // Fetch the block header at the height
    let block_header = client.get_block_header_at(block_height).await?;

    // Fetch timestamps
    let timestamps =
        fetch_block_timestamps_ascending(client, block_height, TIMESTAMPS_FOR_MEDIAN).await?;
    let timestamps: [u32; TIMESTAMPS_FOR_MEDIAN] = timestamps.try_into().expect(
        "fetch_block_timestamps_ascending should return exactly TIMESTAMPS_FOR_MEDIAN timestamps",
    );

    // Compute the block ID for the verified block.
    let block_id: L1BlockId = block_header.block_hash().into();

    // If (block_height + 1) is the start of the new epoch, we need to calculate the
    // next_block_target, else next_block_target will be current block's target
    let next_block_target =
        if (block_height + 1).is_multiple_of(btc_params.difficulty_adjustment_interval()) {
            CompactTarget::from_next_work_required(
                block_header.bits,
                (block_header.time - current_epoch_start_header.time) as u64,
                &btc_params,
            )
            .to_consensus()
        } else {
            client
                .get_block_header_at(block_height)
                .await?
                .target()
                .to_compact_lossy()
                .to_consensus()
        };

    // Build the genesis L1 view structure.
    let genesis_l1_view = GenesisL1View {
        blk: L1BlockCommitment::from_height_u64(block_height, block_id).expect("valid height"),
        next_target: next_block_target,
        epoch_start_timestamp: current_epoch_start_header.time,
        last_11_timestamps: timestamps,
    };

    Ok::<GenesisL1View, anyhow::Error>(genesis_l1_view)
}

/// Retrieves the timestamps for a specified number of blocks starting from the given block height,
/// moving backwards. For each block from `height` down to `height - count + 1`, it fetches the
/// block's timestamp. If a block height is less than 1 (i.e. there is no block), it inserts a
/// placeholder value of 0. The resulting vector is then reversed so that timestamps are returned in
/// ascending order (oldest first).
async fn fetch_block_timestamps_ascending(
    client: &impl Reader,
    height: u64,
    count: usize,
) -> anyhow::Result<Vec<u32>> {
    let mut timestamps = Vec::with_capacity(count);

    for i in 0..count {
        let current_height = height.saturating_sub(i as u64);
        // If we've gone past block 1, push 0 as a placeholder.
        if current_height < 1 {
            timestamps.push(0);
        } else {
            let header = client.get_block_header_at(current_height).await?;
            timestamps.push(header.time);
        }
    }

    timestamps.reverse();
    Ok(timestamps)
}

/// Creates a Bitcoin RPC client from the provided configuration.
fn create_client(config: &BitcoindConfig) -> anyhow::Result<Client> {
    Client::new(
        config.rpc_url.clone(),
        config.rpc_user.clone(),
        config.rpc_password.clone(),
        None,
        None,
    )
    .map_err(|e| anyhow::anyhow!("Failed to create Bitcoin RPC client: {}", e))
}
