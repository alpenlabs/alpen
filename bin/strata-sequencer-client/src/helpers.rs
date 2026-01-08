use std::{fs, path::Path, str::FromStr};

use bitcoin::bip32::Xpriv;
use k256::schnorr::{signature::Signer, Signature, SigningKey};
use ssz::Encode;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_crypto::sign_schnorr_sig;
use strata_key_derivation::sequencer::SequencerKeys;
use strata_ol_chain_types::L2BlockHeader;
use strata_primitives::{
    buf::{Buf32, Buf64},
    keys::ZeroizableXpriv,
};
use strata_sequencer::duty::types::{Identity, IdentityData, IdentityKey};
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

/// Signs the L2BlockHeader and returns the signature
pub(crate) fn sign_header(header: &L2BlockHeader, ik: &IdentityKey) -> Buf64 {
    let msg = header.get_sighash();
    match ik {
        IdentityKey::Sequencer(sk) => sign_schnorr_sig(&msg, sk),
    }
}

/// Signs the new SSZ CheckpointPayload for the SPS-62 checkpoint subprotocol.
///
/// This uses k256 for BIP-340 Schnorr signing of raw SSZ bytes,
/// which is compatible with the predicate framework used by the checkpoint handler.
/// The predicate framework's Bip340Schnorr verifier expects signatures over raw bytes
/// (it handles the tagged hashing internally per BIP-340 spec).
pub(crate) fn sign_checkpoint(payload: &CheckpointPayload, ik: &IdentityKey) -> Buf64 {
    // Sign the SSZ-serialized payload bytes using k256's BIP-340 Schnorr implementation
    // The checkpoint handler's predicate framework verifies against raw SSZ bytes
    let payload_bytes = payload.as_ssz_bytes();
    match ik {
        IdentityKey::Sequencer(sk) => {
            let signing_key =
                SigningKey::from_bytes(sk.as_ref()).expect("sequencer secret key should be valid");
            let signature: Signature = signing_key.sign(&payload_bytes);
            Buf64::from(signature.to_bytes())
        }
    }
}
