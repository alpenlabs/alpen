//! Signing helpers for sequencer duties.

use ssz::Encode;
use strata_asm_checkpoint_types::CheckpointPayload;
use strata_crypto::{hash, sign_schnorr_sig};
use strata_ol_chain_types_v1::OLBlockHeaderV1;
use strata_primitives::buf::{Buf32, Buf64};

/// Signs a [`OLBlockHeaderV1`] and returns the signature.
pub fn sign_header(header: &OLBlockHeaderV1, sk: &Buf32) -> Buf64 {
    let encoded = header.as_ssz_bytes();
    let msg = hash::raw(&encoded);
    sign_schnorr_sig(&msg, sk)
}

/// Signs a [`CheckpointPayload`] and returns the signature.
pub fn sign_checkpoint(checkpoint: &CheckpointPayload, sk: &Buf32) -> Buf64 {
    let encoded = checkpoint.as_ssz_bytes();
    let msg = hash::raw(&encoded);
    sign_schnorr_sig(&msg, sk)
}

/// Signs a reveal transaction sighash and returns the signature.
pub fn sign_reveal_tx(sighash: &Buf32, sk: &Buf32) -> Buf64 {
    sign_schnorr_sig(sighash, sk)
}
