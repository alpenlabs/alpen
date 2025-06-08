use borsh::{BorshDeserialize, BorshSerialize};

use crate::txs::{cancel::CancelAction, enact::EnactAction, updates::UpgradeAction};

/// A high‐level multisig operation that participants can propose.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum MultisigOp {
    /// Cancel a pending action
    Cancel(CancelAction),
    /// Cancel a committed action
    Enact(EnactAction),
    /// Propose an upgrade.
    Upgrade(UpgradeAction),
}

/// A multisig payload comprising an operation plus a nonce, ready for hashing and signing.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct MultisigPayload {
    /// The multisig operation to execute.
    op: MultisigOp,
    /// A strictly increasing nonce to thwart replay.  
    nonce: u64,
}

impl MultisigPayload {
    /// Creates a new multisig payload with the given operation and nonce.
    pub fn new(op: MultisigOp, nonce: u64) -> Self {
        Self { op, nonce }
    }
}

impl From<UpgradeAction> for MultisigOp {
    fn from(upgrade: UpgradeAction) -> Self {
        MultisigOp::Upgrade(upgrade)
    }
}
impl From<CancelAction> for MultisigOp {
    fn from(cancel: CancelAction) -> Self {
        MultisigOp::Cancel(cancel)
    }
}

impl From<EnactAction> for MultisigOp {
    fn from(enact: EnactAction) -> Self {
        MultisigOp::Enact(enact)
    }
}
