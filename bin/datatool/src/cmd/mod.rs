//! Subcommand handlers for the `datatool` binary.

mod alpen_params;
mod asm_params;
mod gen_checkpoint_predicate;
mod genesis_info;
#[cfg(feature = "btc-client")]
mod l1_anchor;
mod ol_params;
mod op_pubkey;
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
        Subcommand::OpPubkey(subc) => op_pubkey::exec(subc, ctx),
        Subcommand::CheckpointPredicate(subc) => gen_checkpoint_predicate::exec(subc, ctx),
        Subcommand::AsmParams(subc) => asm_params::exec(subc, ctx),
        Subcommand::OlParams(subc) => ol_params::exec(subc, ctx),
        Subcommand::AlpenParams(subc) => alpen_params::exec(subc, ctx),
        #[cfg(feature = "btc-client")]
        Subcommand::GenL1Anchor(subc) => l1_anchor::exec(subc, ctx),
    }
}
