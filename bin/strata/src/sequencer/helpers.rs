//! Helpers for sequencer.
//!
//! Mostly related to secure key derivation.

use std::{fs, path::Path, str::FromStr};

use bitcoin::bip32::Xpriv;
use strata_crypto::keys::zeroizable::ZeroizableXpriv;
use strata_key_derivation::sequencer::SequencerKeys;
use strata_primitives::buf::Buf32;
use tracing::debug;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Sequencer key data.
#[derive(Zeroize, ZeroizeOnDrop)]
pub(crate) struct SequencerKey {
    /// Sequencer secret key.
    pub(crate) sk: Buf32,

    /// Sequencer public key.
    pub(crate) pk: Buf32,
}

/// Loads sequencer key from the file at the specified `path`.
pub(crate) fn load_seqkey(path: &Path) -> anyhow::Result<SequencerKey> {
    debug!(?path, "loading sequencer root key");
    let serialized_xpriv = fs::read_to_string(path)?;
    let master_xpriv = ZeroizableXpriv::new(Xpriv::from_str(&serialized_xpriv)?);

    let seq_keys = SequencerKeys::new(&master_xpriv)?;
    let seq_xpriv = seq_keys.derived_xpriv();
    let mut seq_sk = Buf32::from(seq_xpriv.private_key.secret_bytes());
    let seq_xpub = seq_keys.derived_xpub();
    let seq_pk = seq_xpub.to_x_only_pub().serialize();

    let key = SequencerKey {
        sk: seq_sk,
        pk: Buf32::from(seq_pk),
    };

    // I know it's zeroized on drop, but just in case.
    seq_sk.zeroize();

    debug!(pubkey = ?key.pk, "ready to sign as sequencer");
    Ok(key)
}
