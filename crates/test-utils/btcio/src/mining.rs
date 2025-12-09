use bitcoin::{Address, BlockHash};
use bitcoind_async_client::{Client, traits::Wallet};
use corepc_node::Node;

use crate::utils::block_on;

/// Mine a number of blocks of a given size `count`, which may be specified to a given coinbase
/// `address`.
pub async fn mine_blocks(
    bitcoind: &Node,
    client: &Client,
    count: usize,
    address: Option<Address>,
) -> anyhow::Result<Vec<BlockHash>> {
    let coinbase_address = match address {
        Some(address) => address,
        None => client.get_new_address().await?,
    };
    // Use sync client from corepc-node for mining as it is reliable
    let block_hashes = bitcoind
        .client
        .generate_to_address(count as _, &coinbase_address)?
        .0
        .iter()
        .map(|hash: &String| hash.parse::<BlockHash>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(block_hashes)
}

pub fn mine_blocks_blocking(
    bitcoind: &Node,
    client: &Client,
    count: usize,
    address: Option<Address>,
) -> anyhow::Result<Vec<BlockHash>> {
    block_on(mine_blocks(bitcoind, client, count, address))
}
