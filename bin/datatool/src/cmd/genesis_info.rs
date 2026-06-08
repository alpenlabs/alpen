//! Shared genesis-info helpers used by the ASM and OL params generators.

use std::fs;

use alloy_genesis::Genesis;
use alloy_primitives::B256;
use reth_chainspec::ChainSpec;
use strata_btc_verification::L1Anchor;
use strata_primitives::L1Height;

use crate::args::CmdContext;

/// The default L1 genesis height to use.
const DEFAULT_L1_GENESIS_HEIGHT: L1Height = 100;

pub(super) struct BlockInfo {
    pub(super) blockhash: B256,
    pub(super) stateroot: B256,
    pub(super) blocknum: u64,
}

pub(super) fn get_alpen_ee_genesis_block_info(genesis_json: &str) -> anyhow::Result<BlockInfo> {
    let genesis: Genesis = serde_json::from_str(genesis_json)?;

    let chain_spec = ChainSpec::from_genesis(genesis);

    let genesis_header = chain_spec.genesis_header();
    let genesis_stateroot = genesis_header.state_root;
    let genesis_hash = chain_spec.genesis_hash();
    let genesis_blocknum = chain_spec
        .genesis()
        .number
        .expect("genesis block number should be present");

    Ok(BlockInfo {
        blockhash: genesis_hash,
        stateroot: genesis_stateroot,
        blocknum: genesis_blocknum,
    })
}

/// Retrieves the genesis L1 anchor from a file or Bitcoin RPC client.
///
/// Priority:
/// 1. If `l1_anchor_file` is provided, load the [`L1Anchor`] from that JSON file
/// 2. If `btc-client` feature is enabled and RPC credentials are available, fetch from Bitcoin node
/// 3. Otherwise, return an error
pub(super) fn retrieve_l1_anchor(
    l1_anchor_file: Option<&str>,
    genesis_l1_height: Option<L1Height>,
    ctx: &CmdContext,
) -> anyhow::Result<L1Anchor> {
    // Priority 1: Use file if provided
    if let Some(file) = l1_anchor_file {
        let content = fs::read_to_string(file)
            .map_err(|e| anyhow::anyhow!("Failed to read L1 anchor file {:?}: {}", file, e))?;

        let anchor: L1Anchor = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse L1 anchor JSON: {}", e))?;

        return Ok(anchor);
    }

    // Priority 2: Use Bitcoin client if available
    #[cfg(feature = "btc-client")]
    {
        use crate::btc_client::fetch_l1_anchor_with_config;

        if let Some(config) = &ctx.bitcoind_config {
            use tokio::runtime;

            return runtime::Runtime::new()?.block_on(fetch_l1_anchor_with_config(
                config,
                genesis_l1_height.unwrap_or(DEFAULT_L1_GENESIS_HEIGHT),
            ));
        }
    }

    // Priority 3: Return error if neither option is available
    Err(anyhow::anyhow!(
        "Either provide --l1-anchor-file or specify Bitcoin RPC credentials (--bitcoin-rpc-url, --bitcoin-rpc-user, --bitcoin-rpc-password) when btc-client feature is enabled"
    ))
}
