//! `genoppubkey` subcommand: derives an operator compressed public key from a master xpriv.

use std::{fs, str::FromStr};

use bitcoin::{bip32::Xpriv, secp256k1::SECP256K1};

use crate::args::{CmdContext, SubcOpPubkey};

/// Executes the `genoppubkey` subcommand.
///
/// Reads a master [`Xpriv`] from the specified file and prints the corresponding
/// compressed public key (33-byte hex) to stdout.
pub(super) fn exec(cmd: SubcOpPubkey, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let xpriv = Xpriv::from_str(fs::read_to_string(&cmd.key_file)?.trim())?;
    let pubkey = xpriv.to_keypair(SECP256K1).public_key();
    println!("{pubkey}");

    Ok(())
}
