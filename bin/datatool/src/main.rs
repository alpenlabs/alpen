//! Command line tool for generating test data for Strata.
//!
//! # Warning
//!
//! This tool is intended for use in testing and development only. It generates
//! keys and other data that should not be used in production.

#[cfg(feature = "risc0-builder")]
use bytemuck as _;
#[cfg(feature = "risc0-builder")]
use risc0_groth16 as _;
#[cfg(feature = "risc0-builder")]
use risc0_zkvm as _;
#[cfg(feature = "sp1-builder")]
use sp1_verifier as _;
#[cfg(feature = "risc0-builder")]
use strata_risc0_guest_builder as _;
#[cfg(feature = "risc0-builder")]
use zkaleido_risc0_groth16_verifier as _;
#[cfg(feature = "sp1-builder")]
use zkaleido_sp1_groth16_verifier as _;

mod args;
mod util;

use std::path::PathBuf;

use args::CmdContext;
use rand_core::OsRng;
use util::{exec_subc, resolve_network};

fn main() {
    let args: args::Args = argh::from_env();
    if let Err(e) = main_inner(args) {
        eprintln!("ERROR\n{e:?}");
    }
}

fn main_inner(args: args::Args) -> anyhow::Result<()> {
    let network = resolve_network(args.bitcoin_network.as_deref())?;

    let mut ctx = CmdContext {
        datadir: args.datadir.unwrap_or_else(|| PathBuf::from(".")),
        bitcoin_network: network,
        rng: OsRng,
    };

    exec_subc(args.subc, &mut ctx)?;
    Ok(())
}
