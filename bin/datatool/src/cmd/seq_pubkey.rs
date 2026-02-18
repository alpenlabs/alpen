//! `genseqpubkey` subcommand: generates a sequencer pubkey from a master xpriv.

use strata_key_derivation::sequencer::SequencerKeys;

use crate::{
    args::{CmdContext, SubcSeqPubkey},
    util::{resolve_xpriv, SEQKEY_ENVVAR},
};

/// Executes the `genseqpubkey` subcommand.
///
/// Generates the sequencer [`Xpub`](bitcoin::bip32::Xpub) from the provided
/// [`Xpriv`](bitcoin::bip32::Xpriv) and prints it to stdout.
pub(super) fn exec(cmd: SubcSeqPubkey, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let Some(xpriv) = resolve_xpriv(&cmd.key_file, cmd.key_from_env, SEQKEY_ENVVAR)? else {
        anyhow::bail!("privkey unset");
    };

    let seq_keys = SequencerKeys::new(&xpriv)?;
    let seq_xpub = seq_keys.derived_xpub();
    println!("{seq_xpub}");

    Ok(())
}
