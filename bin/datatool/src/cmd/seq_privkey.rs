//! `genseqprivkey` subcommand: generates a sequencer privkey from a master xpriv.

use strata_key_derivation::sequencer::SequencerKeys;

use crate::{
    args::{CmdContext, SubcSeqPrivkey},
    util::{resolve_xpriv, SEQKEY_ENVVAR},
};

/// Executes the `genseqprivkey` subcommand.
///
/// Generates the sequencer [`Xpriv`](bitcoin::bip32::Xpriv) that will
/// [`Zeroize`](zeroize::Zeroize) on [`Drop`] and prints it to stdout.
pub(super) fn exec(cmd: SubcSeqPrivkey, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let Some(xpriv) = resolve_xpriv(&cmd.key_file, cmd.key_from_env, SEQKEY_ENVVAR)? else {
        anyhow::bail!("privkey unset");
    };

    let seq_keys = SequencerKeys::new(&xpriv)?;
    let seq_xpriv = seq_keys.derived_xpriv();
    println!("{seq_xpriv}");

    Ok(())
}
