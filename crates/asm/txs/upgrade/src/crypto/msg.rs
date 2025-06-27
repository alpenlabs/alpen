use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

use crate::actions::MultisigAction;

/// A multisig payload comprising an operation plus a nonce, ready for hashing and signing.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct MultisigPayload {
    op: MultisigAction,
    nonce: u64,
}

impl MultisigPayload {
    /// Create a new multisig payload.
    pub fn new(op: MultisigAction, nonce: u64) -> Self {
        Self { op, nonce }
    }

    /// Borrow the multisig operation.
    pub fn op(&self) -> &MultisigAction {
        &self.op
    }

    /// The nonce associated with this payload.
    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Consume and return the inner `(MultisigOp, u64)`.
    pub fn into_inner(self) -> (MultisigAction, u64) {
        (self.op, self.nonce)
    }
}
