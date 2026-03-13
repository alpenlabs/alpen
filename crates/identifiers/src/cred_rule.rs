//! Types relating to block credentials and signing.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};

use crate::Buf32;

/// Rule we use to decide how to identify if an L2 block is correctly signed.
#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    BorshSerialize,
    BorshDeserialize,
    DeriveEncode,
    DeriveDecode,
)]
#[serde(rename_all = "snake_case")]
#[ssz(enum_behaviour = "union")]
pub enum CredRule {
    /// Any block gets accepted, unconditionally.
    Unchecked,

    /// Just sign every block with a static BIP340 schnorr pubkey.
    SchnorrKey(Buf32),
}
