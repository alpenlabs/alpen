use std::env;

use bitcoin::{absolute, block::Header, Address};
use bitcoind_async_client::{
    traits::{Reader, Wallet},
    Client,
};
use corepc_node::Node;
use strata_btc_types::GenesisL1View;
use strata_identifiers::{L1BlockCommitment, L1BlockId};

/// Get the authentication credentials for a given `bitcoind` instance.
fn get_auth(bitcoind: &Node) -> (String, String) {
    let params = &bitcoind.params;
    let cookie_values = params.get_cookie_values().unwrap().unwrap();
    (cookie_values.user, cookie_values.password)
}

pub fn get_bitcoind_and_client() -> (Node, Client) {
    // setting the ENV variable `BITCOIN_XPRIV_RETRIEVABLE` to retrieve the xpriv
    // SAFETY: This is a test environment and we control the execution flow.
    unsafe {
        env::set_var("BITCOIN_XPRIV_RETRIEVABLE", "true");
    }
    let bitcoind = Node::new("bitcoind").unwrap();
    let url = bitcoind.rpc_url();
    let (user, password) = get_auth(&bitcoind);
    let client = Client::new(url, user, password, None, None).unwrap();
    (bitcoind, client)
}

/// Mine a number of blocks of a given size `count`, which may be specified to a given coinbase
/// `address`.
pub async fn mine_blocks(
    bitcoind: &Node,
    client: &Client,
    count: usize,
    address: Option<Address>,
) -> anyhow::Result<Vec<bitcoin::BlockHash>> {
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
        .map(|hash: &String| hash.parse::<bitcoin::BlockHash>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(block_hashes)
}

/// Helper to construct `GenesisL1View` from a block hash using the client.
pub async fn get_genesis_l1_view(
    client: &Client,
    hash: &bitcoin::BlockHash,
) -> anyhow::Result<GenesisL1View> {
    let header: Header = client.get_block_header(hash).await?;
    let height = client.get_block_height(hash).await?;

    // Construct L1BlockCommitment
    let blkid: L1BlockId = header.block_hash().into();
    let blk_commitment = L1BlockCommitment::new(
        absolute::Height::from_consensus(height as u32).expect("Height u32 overflow"),
        blkid,
    );

    // Create dummy/default values for other fields
    let next_target = header.bits.to_consensus();
    let epoch_start_timestamp = header.time;
    let last_11_timestamps = [header.time - 1; 11]; // simplified: ensure median < tip time by making history older

    Ok(GenesisL1View {
        blk: blk_commitment,
        next_target,
        epoch_start_timestamp,
        last_11_timestamps, // simplified: ensure median < tip time by making history older
    })
}
