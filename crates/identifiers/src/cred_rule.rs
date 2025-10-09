use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::Buf32;

/// Credential rule for block validation
#[derive(
    Clone, Debug, PartialEq, Eq, Arbitrary, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub enum CredRule {
    /// No credential checking
    Unchecked,
    /// Schnorr signature verification
    SchnorrKey(Buf32),
}
