use borsh::{BorshDeserialize, BorshSerialize};

use crate::txs::{cancel::CancelAction, enact::EnactAction, updates::UpgradeAction};

/// A high‐level multisig operation that participants can propose.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum MultisigOp {
    /// Cancel a pending action
    Cancel(CancelAction),
    /// Execute a committed action
    Enact(EnactAction),
    /// Propose an upgrade
    Upgrade(UpgradeAction),
}

/// A multisig payload comprising an operation plus a nonce, ready for hashing and signing.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigPayload {
    op: MultisigOp,
    nonce: u64,
}

impl MultisigPayload {
    /// Create a new multisig payload.
    pub fn new(op: MultisigOp, nonce: u64) -> Self {
        Self { op, nonce }
    }

    /// Borrow the multisig operation.
    pub fn op(&self) -> &MultisigOp {
        &self.op
    }

    /// The nonce associated with this payload.
    pub fn nonce(&self) -> u64 {
        self.nonce
    }

    /// Consume and return the inner `(MultisigOp, u64)`.
    pub fn into_inner(self) -> (MultisigOp, u64) {
        (self.op, self.nonce)
    }
}

// Allow constructing a `MultisigOp` from each action type
impl From<UpgradeAction> for MultisigOp {
    fn from(action: UpgradeAction) -> Self {
        MultisigOp::Upgrade(action)
    }
}

impl From<CancelAction> for MultisigOp {
    fn from(action: CancelAction) -> Self {
        MultisigOp::Cancel(action)
    }
}

impl From<EnactAction> for MultisigOp {
    fn from(action: EnactAction) -> Self {
        MultisigOp::Enact(action)
    }
}
