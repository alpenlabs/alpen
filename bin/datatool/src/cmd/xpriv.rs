//! `genxpriv` subcommand: generates a master xpriv and writes it to a file.

use std::fs;

use bitcoin::{bip32::Xpriv, Network};
use rand_core::CryptoRngCore;
use strata_crypto::keys::zeroizable::ZeroizableXpriv;
use zeroize::Zeroize;

use crate::args::{CmdContext, SubcXpriv};

/// Executes the `genxpriv` subcommand.
///
/// Generates a new [`Xpriv`] that will [`Zeroize`](zeroize) on [`Drop`] and writes it to a file.
pub(super) fn exec(cmd: SubcXpriv, ctx: &mut CmdContext) -> anyhow::Result<()> {
    if cmd.path.exists() && !cmd.force {
        anyhow::bail!("not overwriting file, add --force to overwrite");
    }

    let xpriv = gen_priv(&mut ctx.rng, ctx.bitcoin_network);

    let result = fs::write(&cmd.path, xpriv.to_string().as_bytes());

    match result {
        Ok(_) => Ok(()),
        Err(_) => anyhow::bail!("failed to write to file {:?}", cmd.path),
    }
}

/// Generates a new [`Xpriv`] that will [`Zeroize`](zeroize) on [`Drop`].
///
/// Takes a mutable reference to an RNG to allow flexibility in testing.
/// The actual generation requires a high-entropy source like [`OsRng`](rand_core::OsRng)
/// to securely generate extended private keys.
fn gen_priv<R: CryptoRngCore>(rng: &mut R, net: Network) -> ZeroizableXpriv {
    let mut seed = [0u8; 32];
    rng.fill_bytes(&mut seed);
    let mut xpriv = Xpriv::new_master(net, &seed).expect("valid seed");
    let zeroizable_xpriv: ZeroizableXpriv = xpriv.into();

    // Zeroize the seed after generating the xpriv.
    seed.zeroize();
    // Zeroize the xpriv after generating it.
    //
    // NOTE: `zeroizable_xpriv` is zeroized on drop.
    xpriv.private_key.non_secure_erase();

    zeroizable_xpriv
}
