//! `gen-l1-anchor` subcommand: generates the genesis L1 anchor at the given height.

use std::fs;

use tokio::runtime;

use crate::{
    args::{CmdContext, SubcGenL1Anchor},
    btc_client::fetch_l1_anchor_with_config,
};

/// Executes the `gen-l1-anchor` subcommand.
///
/// Fetches the genesis L1 anchor from a Bitcoin node at the specified height.
pub(super) fn exec(cmd: SubcGenL1Anchor, ctx: &mut CmdContext) -> anyhow::Result<()> {
    let config = ctx
        .bitcoind_config
        .as_ref()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Bitcoin RPC configuration not provided. Please specify --bitcoin-rpc-url, --bitcoin-rpc-user, and --bitcoin-rpc-password"
            )
        })?;

    let anchor = runtime::Runtime::new()?
        .block_on(fetch_l1_anchor_with_config(config, cmd.genesis_l1_height))?;

    let params_buf = serde_json::to_string_pretty(&anchor)?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    Ok(())
}
