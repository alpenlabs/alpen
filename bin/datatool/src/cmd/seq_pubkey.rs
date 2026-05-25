//! `genseqpubkey` subcommand: derives the sequencer x-only public key from a master xpriv.

use strata_key_derivation::sequencer::SequencerKeys;

use crate::{
    args::{CmdContext, SubcSeqPubkey},
    util::{resolve_xpriv, SEQKEY_ENVVAR},
};

/// Executes the `genseqpubkey` subcommand.
///
/// Derives the sequencer x-only public key (32-byte hex) from the provided
/// master [`Xpriv`](bitcoin::bip32::Xpriv) and prints it to stdout.
pub(super) fn exec(cmd: SubcSeqPubkey, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let Some(xpriv) = resolve_xpriv(&cmd.key_file, cmd.key_from_env, SEQKEY_ENVVAR)? else {
        anyhow::bail!("privkey unset");
    };

    let seq_keys = SequencerKeys::new(&xpriv)?;
    let xonly = seq_keys.derived_xpub().to_x_only_pub();
    println!("{xonly}");

    Ok(())
}
