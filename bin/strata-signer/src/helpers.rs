//! Helpers for sequencer signer key loading.

use std::{fs, path::Path, str::FromStr, sync::Arc};

use bitcoin::bip32::Xpriv;
use strata_crypto::keys::zeroizable::{ZeroizableXpriv, ZeroizedBuf32};
use strata_key_derivation::sequencer::SequencerKeys;
use strata_primitives::buf::Buf32;
use strata_signer::helpers::SequencerSk;
use tracing::debug;
use zeroize::Zeroize;

/// Loads the sequencer key from the file at `path`.
///
/// Returns the secret key as a [`SequencerSk`] and the corresponding public
/// key as a [`Buf32`]. The raw secret bytes are zeroized before this function
/// returns.
pub(crate) fn load_seqkey(path: &Path) -> anyhow::Result<(SequencerSk, Buf32)> {
    debug!(?path, "loading sequencer root key");
    let mut serialized_xpriv = fs::read_to_string(path)?;
    let master_xpriv = ZeroizableXpriv::new(Xpriv::from_str(&serialized_xpriv)?);
    serialized_xpriv.zeroize();

    let seq_keys = SequencerKeys::new(&master_xpriv)?;
    let seq_xpriv = seq_keys.derived_xpriv();
    let mut raw_sk: [u8; 32] = seq_xpriv.private_key.secret_bytes();
    let seq_xpub = seq_keys.derived_xpub();
    let seq_pk = Buf32::from(seq_xpub.to_x_only_pub().serialize());

    let sk = Arc::new(ZeroizedBuf32::new(raw_sk));
    raw_sk.zeroize();

    debug!(pubkey = ?seq_pk, "ready to sign as sequencer");
    Ok((sk, seq_pk))
}
