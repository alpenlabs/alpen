//! Types relating to block credentials and signing.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use strata_identifiers::Buf32;

/// Rule we use to decide how to identify if an L2 block is correctly signed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, BorshSerialize, BorshDeserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredRule {
    /// Any block gets accepted, unconditionally.
    Unchecked,

    /// Just sign every block with a static BIP340 schnorr pubkey.
    SchnorrKey(Buf32),
}
