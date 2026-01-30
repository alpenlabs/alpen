use std::{fs, path::Path, str::FromStr};

use bitcoin::bip32::Xpriv;
use ssz::Encode;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_codec::encode_to_vec;
use strata_codec_utils::CodecSsz;
use strata_crypto::{hash, keys::zeroizable::ZeroizableXpriv, sign_schnorr_sig};
use strata_key_derivation::sequencer::SequencerKeys;
use strata_ol_chain_types_new::OLBlockHeader;
use strata_primitives::buf::{Buf32, Buf64};
use strata_sequencer_new::duty::types::{Identity, IdentityData, IdentityKey};
use tracing::debug;
use zeroize::Zeroize;

/// Loads sequencer identity data from the root key at the specified path.
pub(crate) fn load_seqkey(path: &Path) -> anyhow::Result<IdentityData> {
    debug!(?path, "loading sequencer root key");
    let serialized_xpriv = fs::read_to_string(path)?;
    let master_xpriv = ZeroizableXpriv::new(Xpriv::from_str(&serialized_xpriv)?);

    // Actually do the key derivation from the root key and then derive the pubkey from that.
    let seq_keys = SequencerKeys::new(&master_xpriv)?;
    let seq_xpriv = seq_keys.derived_xpriv();
    let mut seq_sk = Buf32::from(seq_xpriv.private_key.secret_bytes());
    let seq_xpub = seq_keys.derived_xpub();
    let seq_pk = seq_xpub.to_x_only_pub().serialize();

    let ik = IdentityKey::Sequencer(seq_sk);
    let ident = Identity::Sequencer(Buf32::from(seq_pk));

    // Zeroize the Buf32 representation of the Xpriv.
    seq_sk.zeroize();

    // Changed this to the pubkey so that we don't just log our privkey.
    debug!(?ident, "ready to sign as sequencer");

    let idata = IdentityData::new(ident, ik);
    Ok(idata)
}

/// Sign OL block header using Schnorr signature.
pub(crate) fn sign_ol_header(header: &OLBlockHeader, ik: &IdentityKey) -> Buf64 {
    let encoded =
        encode_to_vec(&CodecSsz::new(header.clone())).expect("codec encoding should succeed");
    let msg = hash::raw(&encoded);
    match ik {
        IdentityKey::Sequencer(sk) => sign_schnorr_sig(&msg, sk),
    }
}

/// Sign checkpoint payload.
pub(crate) fn sign_checkpoint_payload(payload: &CheckpointPayload, ik: &IdentityKey) -> Buf64 {
    let encoded = payload.as_ssz_bytes();
    let msg = hash::raw(&encoded);
    match ik {
        IdentityKey::Sequencer(sk) => sign_schnorr_sig(&msg, sk),
    }
}
