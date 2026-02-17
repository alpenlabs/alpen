//! Subcommand handlers for the `datatool` binary.

#[cfg(feature = "btc-client")]
mod l1_view;
mod params;
mod seq_privkey;
mod seq_pubkey;
mod xpriv;

use crate::args::{CmdContext, Subcommand};

/// Executes a subcommand.
pub(crate) fn exec_subc(cmd: Subcommand, ctx: &mut CmdContext) -> anyhow::Result<()> {
    match cmd {
        Subcommand::Xpriv(subc) => xpriv::exec(subc, ctx),
        Subcommand::SeqPubkey(subc) => seq_pubkey::exec(subc, ctx),
        Subcommand::SeqPrivkey(subc) => seq_privkey::exec(subc, ctx),
        Subcommand::Params(subc) => params::exec(subc, ctx),
        #[cfg(feature = "btc-client")]
        Subcommand::GenL1View(subc) => l1_view::exec(subc, ctx),
    }
}
