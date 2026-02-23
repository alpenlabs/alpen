//! `gen-ol-params` subcommand: generates OL params from inputs.

use std::fs;

use strata_ol_params::OLParams;

use crate::args::{CmdContext, SubcOlParams};

/// Executes the `gen-ol-params` subcommand.
///
/// Generates the OL params for a Strata network.
/// Either writes to a file or prints to stdout depending on the provided options.
pub(super) fn exec(cmd: SubcOlParams, ctx: &mut CmdContext) -> anyhow::Result<()> {
    let genesis_l1_view = super::params::retrieve_genesis_l1_view(
        cmd.genesis_l1_view_file.as_deref(),
        cmd.genesis_l1_height,
        ctx,
    )?;

    let ol_params = OLParams::new_empty(genesis_l1_view.blk);

    let params_buf = serde_json::to_string_pretty(&ol_params)?;

    if let Some(out_path) = &cmd.output {
        fs::write(out_path, &params_buf)?;
        eprintln!("wrote to file {out_path:?}");
    } else {
        println!("{params_buf}");
    }

    Ok(())
}
