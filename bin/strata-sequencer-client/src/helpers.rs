use std::{fs, path::Path};

use bitcoin::{base58, bip32::Xpriv};
use strata_crypto::sign_schnorr_sig;
use strata_key_derivation::sequencer::SequencerKeys;
use strata_primitives::{
    buf::{Buf32, Buf64},
    keys::ZeroizableXpriv,
};
use strata_sequencer::duty::types::{Identity, IdentityData, IdentityKey};
use strata_state::{batch::BatchCheckpoint, header::L2BlockHeader};
use tracing::debug;
use zeroize::Zeroize;

/// Loads sequencer identity data from the root key at the specified path.
pub(crate) fn load_seqkey(path: &Path) -> anyhow::Result<IdentityData> {
    let raw_buf = fs::read(path)?;
    let str_buf = std::str::from_utf8(&raw_buf)?;
    debug!(?path, "loading sequencer root key");
    let buf = base58::decode_check(str_buf)?;
    let master_xpriv = ZeroizableXpriv::new(Xpriv::decode(&buf)?);

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

/// Signs the L2BlockHeader and returns the signature
pub(crate) fn sign_header(header: &L2BlockHeader, ik: &IdentityKey) -> Buf64 {
    let msg = header.get_sighash();
    match ik {
        IdentityKey::Sequencer(sk) => sign_schnorr_sig(&msg, sk),
    }
}

pub(crate) fn sign_checkpoint(checkpoint: &BatchCheckpoint, ik: &IdentityKey) -> Buf64 {
    let msg = checkpoint.hash();
    match ik {
        IdentityKey::Sequencer(sk) => sign_schnorr_sig(&msg, sk),
    }
}
