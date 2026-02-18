//! Command line tool for generating test data for Strata.
//!
//! # Warning
//!
//! This tool is intended for use in testing and development only. It generates
//! keys and other data that should not be used in production.

#[cfg(feature = "sp1-builder")]
use sp1_verifier as _;
use strata_asm_types as _;
#[cfg(feature = "sp1-builder")]
use zkaleido_sp1_groth16_verifier as _;

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
