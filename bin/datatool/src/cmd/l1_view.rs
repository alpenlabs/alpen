//! `genl1view` subcommand: generates the genesis L1 view at the given height.

use std::fs;

use tokio::runtime;

use crate::{
    args::{CmdContext, SubcGenL1View},
    btc_client::fetch_genesis_l1_view_with_config,
};

/// Executes the `genl1view` subcommand.
///
/// Fetches the genesis L1 view from a Bitcoin node at the specified height.
pub(super) fn exec(cmd: SubcGenL1View, ctx: &mut CmdContext) -> anyhow::Result<()> {
    let config = ctx
        .bitcoind_config
        .as_ref()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Bitcoin RPC configuration not provided. Please specify --bitcoin-rpc-url, --bitcoin-rpc-user, and --bitcoin-rpc-password"
            )
        })?;

    let gl1view = runtime::Runtime::new()?.block_on(fetch_genesis_l1_view_with_config(
        config,
        cmd.genesis_l1_height,
    ))?;

    let params_buf = serde_json::to_string_pretty(&gl1view)?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    Ok(())
}
