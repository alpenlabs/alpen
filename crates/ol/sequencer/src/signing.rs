//! Signing helpers for sequencer duties.

use ssz::Encode;
use strata_checkpoint_types_ssz::CheckpointPayload;
use strata_crypto::{hash, sign_schnorr_sig};
use strata_ol_chain_types_new::OLBlockHeader;
use strata_primitives::buf::{Buf32, Buf64};

/// Signs a [`OLBlockHeader`] and returns the signature.
pub fn sign_header(header: &OLBlockHeader, sk: &Buf32) -> Buf64 {
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
