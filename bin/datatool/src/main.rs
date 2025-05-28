//! Command line tool for generating test data for Strata.
//!
//! # Warning
//!
//! This tool is intended for use in testing and development only. It generates
//! keys and other data that should not be used in production.
mod args;
mod util;

use std::path::PathBuf;

use args::CmdContext;
use rand_core::OsRng;
use util::{exec_subc, resolve_network};

fn main() {
    let args: args::Args = argh::from_env();
    let inner = || -> anyhow::Result<()> {
        let network = resolve_network(args.bitcoin_network.as_deref())?;

        let mut ctx = CmdContext {
            datadir: args.datadir.unwrap_or_else(|| PathBuf::from(".")),
            bitcoin_network: network,
            rng: OsRng,
        };

        exec_subc(args.subc, &mut ctx)?;
        Ok(())
    };
    if let Err(e) = inner() {
        eprintln!("ERROR\n{e:?}");
    }
}
