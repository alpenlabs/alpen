//! Command line tool for generating test data for Strata.
//!
//! # Warning
//!
//! This tool is intended for use in testing and development only. It generates
//! keys and other data that should not be used in production.

use strata_btc_verification as _;

mod acct_predicate;
mod args;
#[cfg(feature = "btc-client")]
mod btc_client;
mod checkpoint_predicate;
mod cmd;
mod util;

use args::resolve_context_and_subcommand;
use cmd::exec_subc;

fn main() {
    let args: args::Args = argh::from_env();
    let inner = || -> anyhow::Result<()> {
        let (mut ctx, subc) = resolve_context_and_subcommand(args)?;
        exec_subc(subc, &mut ctx)?;
        Ok(())
    };
    if let Err(e) = inner() {
        eprintln!("ERROR\n{e:?}");
    }
}
