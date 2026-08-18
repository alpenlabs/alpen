//! Test utilities and proptest strategies for OL chain types.
//!
//! This module contains reusable test utilities and proptest strategies that are used
//! across multiple test modules to avoid code duplication.

#![allow(unreachable_pub, reason = "test utils module")]

use proptest::prelude::*;
use rand::RngCore;
use secp256k1::{Keypair, SECP256K1};
use strata_identifiers::test_utils::{buf32_strategy, buf64_strategy, ol_block_id_strategy};
use strata_identifiers::{AccountSerial, Buf32, Epoch, Slot};
use strata_ol_tx_types_v1::test_utils::ol_transaction_strategy;
pub use strata_ol_tx_types_v1::test_utils::schnorr_predicate;

use crate::block_flags::BlockFlagsV1;
use crate::ssz_generated::ssz::block::*;
use crate::*;

/// Generates a random, valid BIP-340 Schnorr keypair as `(secret_key, x_only_pubkey)` for tests.
///
/// The public key is the x-only key derived from the secret key, so signatures produced over the
/// secret key with `sign_schnorr_sig` verify against a [`schnorr_predicate`] built from the public
/// key.
pub fn test_schnorr_keypair() -> (Buf32, Buf32) {
    // Mirror `EvenPublicKey`'s `Arbitrary` impl: clamp the random bytes so the scalar is always
    // below the secp256k1 curve order (which starts with `0xFF`) and non-zero, making the secret
    // key infallibly valid.
    let mut sk_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut sk_bytes);
    sk_bytes[0] &= 0xFE;
    sk_bytes[31] |= 1;

    let keypair = Keypair::from_seckey_slice(SECP256K1, &sk_bytes)
        .expect("clamped bytes are always a valid secret key");
    let sk = Buf32::from(keypair.secret_bytes());
    let pk = Buf32::from(keypair.x_only_public_key().0.serialize());
    (sk, pk)
}

/// Strategy for generating random [`OLLog`] values.
pub fn ol_log_strategy() -> impl Strategy<Value = OLLog> {
    (
        any::<u32>().prop_map(AccountSerial::from),
        prop::collection::vec(any::<u8>(), 0..1024),
    )
        .prop_map(|(account_serial, payload)| OLLog::new(account_serial, payload))
}

pub fn ol_tx_segment_strategy() -> impl Strategy<Value = OLTxSegmentV1> {
    prop::collection::vec(ol_transaction_strategy(), 0..10).prop_map(|txs| OLTxSegmentV1 {
        txs: txs
            .try_into()
            .expect("transactions must fit within SSZ max length"),
    })
}

pub fn manifests_strategy() -> impl Strategy<Value = Option<OLAsmManifestContainerV1>> {
    prop::option::of(Just(
        OLAsmManifestContainerV1::new(vec![]).expect("empty manifest should succeed"),
    ))
}

pub fn ol_block_header_strategy() -> impl Strategy<Value = OLBlockHeaderV1> {
    (
        any::<u64>(),
        any::<u16>().prop_map(BlockFlagsV1::from),
        any::<Slot>(),
        any::<Epoch>(),
        ol_block_id_strategy(),
        buf32_strategy(),
        buf32_strategy(),
        buf32_strategy(),
    )
        .prop_map(
            |(timestamp, flags, slot, epoch, parent_blkid, body_root, state_root, logs_root)| {
                OLBlockHeaderV1 {
                    timestamp,
                    flags,
                    slot,
                    epoch,
                    parent_blkid,
                    body_root,
                    state_root,
                    logs_root,
                }
            },
        )
}

pub fn signed_ol_block_header_strategy() -> impl Strategy<Value = SignedOLBlockHeaderV1> {
    (ol_block_header_strategy(), buf64_strategy()).prop_map(|(header, signature)| {
        SignedOLBlockHeaderV1 {
            header,
            credential: OLBlockCredentialV1 {
                schnorr_sig: Some(signature).into(),
            },
        }
    })
}

pub fn ol_block_body_strategy() -> impl Strategy<Value = OLBlockBodyV1> {
    (ol_tx_segment_strategy(), manifests_strategy()).prop_map(|(tx_segment, manifests)| {
        OLBlockBodyV1 {
            tx_segment: Some(tx_segment).into(),
            manifests: manifests.into(),
        }
    })
}

pub fn ol_block_strategy() -> impl Strategy<Value = OLBlockV1> {
    (signed_ol_block_header_strategy(), ol_block_body_strategy()).prop_map(
        |(signed_header, body)| OLBlockV1 {
            signed_header,
            body,
        },
    )
}
