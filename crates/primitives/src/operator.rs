use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use super::bridge::OperatorIdx;
use crate::prelude::Buf32;

/// Some type that can provide operator keys.
pub trait OperatorKeyProvider {
    /// Returns the operator's signing pubkey, if it exists in the table.
    fn get_operator_signing_pk(&self, idx: OperatorIdx) -> Option<Buf32>;
}

/// Stub key provider that can be used for testing.
#[derive(Debug)]
pub struct StubOpKeyProv {
    expected_idx: OperatorIdx,
    pk: Buf32,
}

impl StubOpKeyProv {
    pub fn new(expected_idx: OperatorIdx, pk: Buf32) -> Self {
        Self { expected_idx, pk }
    }
}

impl OperatorKeyProvider for StubOpKeyProv {
    fn get_operator_signing_pk(&self, idx: OperatorIdx) -> Option<Buf32> {
        if idx == self.expected_idx {
            Some(self.pk)
        } else {
            None
        }
    }
}

/// Container for operator pubkeys.
#[derive(Clone, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, Deserialize, Serialize)]
pub struct OperatorPubkeys {
    signing_pk: Buf32,
    wallet_pk: Buf32,
}

impl OperatorPubkeys {
    pub fn new(signing_pk: Buf32, wallet_pk: Buf32) -> Self {
        Self {
            signing_pk,
            wallet_pk,
        }
    }

    pub const fn signing_pk(&self) -> &Buf32 {
        &self.signing_pk
    }

    pub const fn wallet_pk(&self) -> &Buf32 {
        &self.wallet_pk
    }
}
