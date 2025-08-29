//! Bitcoin client functionality for Strata `datatool` binary.
//!
//! This module contains Bitcoin RPC client operations and is feature-gated
//! behind the `btc-client` feature flag.

use anyhow;
#[cfg(feature = "btc-client")]
use bitcoin::CompactTarget;
#[cfg(feature = "btc-client")]
use bitcoind_async_client::{traits::Reader, Client};
#[cfg(feature = "btc-client")]
use strata_primitives::l1::{
    get_relative_difficulty_adjustment_height, BtcParams, L1BlockCommitment, L1BlockId,
    TIMESTAMPS_FOR_MEDIAN,
};
use strata_primitives::params::GenesisL1View;

use crate::args::SubcParams;

/// Retrieves the genesis L1 view either from a Bitcoin RPC client or from a local file.
///
/// When the `btc-client` feature is enabled, this function connects to a Bitcoin node
/// using the RPC credentials from `cmd` and fetches the genesis L1 view at the specified
/// block height (defaults to 100 if not provided).
///
/// When the `btc-client` feature is disabled, the function loads the genesis L1 view
/// from a JSON file specified in `cmd.genesis_l1_view_file`.
///
/// # Arguments
/// * `cmd` - Command parameters containing Bitcoin RPC connection details and file paths
///
/// # Returns
/// * `Ok(GenesisL1View)` - The successfully retrieved genesis L1 view
/// * `Err(anyhow::Error)` - If RPC connection fails, file reading fails, or JSON parsing fails
pub(crate) fn get_genesis_l1_view(cmd: &SubcParams) -> anyhow::Result<GenesisL1View> {
    #[cfg(feature = "btc-client")]
    {
        // When the btc-client feature is enabled, we can fetch the genesis L1 view from a Bitcoin
        // node.
        let bitcoin_client = Client::new(
            cmd.bitcoin_rpc_url.clone(),
            cmd.bitcoin_rpc_user.clone(),
            cmd.bitcoin_rpc_password.clone(),
            None,
            None,
        )
        .map_err(|e| anyhow::anyhow!("Failed to create Bitcoin RPC client: {}", e))?;

        tokio::runtime::Runtime::new()?.block_on(fetch_genesis_l1_view(
            &bitcoin_client,
            cmd.genesis_l1_height.unwrap_or(100),
        ))
    }

    #[cfg(not(feature = "btc-client"))]
    {
        // When the btc-client feature is disabled, we can only load the genesis L1 view from a
        // file.
        use std::fs;
        
        let content = fs::read_to_string(&cmd.genesis_l1_view_file)
            .map_err(|e| anyhow::anyhow!("Failed to read genesis L1 view file {:?}: {}", cmd.genesis_l1_view_file, e))?;

        let genesis_l1_view: GenesisL1View = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse genesis L1 view JSON: {}", e))?;

        Ok(genesis_l1_view)
    }
}

#[cfg(feature = "btc-client")]
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
        blk: L1BlockCommitment::new(block_height, block_id),
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
#[cfg(feature = "btc-client")]
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