//! Forced inclusion types.
//!
//! This is all stubs now so that we can define data structures later.

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

use crate::l1;

#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct ForcedInclusion {
    payload: Vec<u8>,
}

impl ForcedInclusion {
    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct ForcedInclusionTx {
    tx: l1::L1Tx,
}
