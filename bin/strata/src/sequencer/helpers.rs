//! Helpers for sequencer.
//!
//! Mostly related to secure key derivation and signing.

use std::{fs, path::Path, str::FromStr};

use bitcoin::bip32::Xpriv;
use ssz::Encode;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_crypto::{hash, keys::zeroizable::ZeroizableXpriv, sign_schnorr_sig};
use strata_key_derivation::sequencer::SequencerKeys;
use strata_ol_chain_types_new::OLBlockHeader;
use strata_primitives::buf::{Buf32, Buf64};
use tracing::debug;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Sequencer key data.
#[derive(ZeroizeOnDrop)]
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

/// Signs a [`OLBlockHeader`] and returns the signature.
pub(crate) fn sign_header(header: &OLBlockHeader, sk: &Buf32) -> Buf64 {
    let encoded = header.as_ssz_bytes();
    let msg = hash::raw(&encoded);
    sign_schnorr_sig(&msg, sk)
}

/// Signs a [`CheckpointPayload`] and returns the signature.
pub(crate) fn sign_checkpoint(checkpoint: &CheckpointPayload, sk: &Buf32) -> Buf64 {
    let encoded = checkpoint.as_ssz_bytes();
    let msg = hash::raw(&encoded);
    sign_schnorr_sig(&msg, sk)
}
