use borsh::{BorshDeserialize, BorshSerialize};

mod cancel;
mod enact;
pub mod updates;

pub use cancel::CancelAction;
pub use enact::EnactAction;
pub use updates::UpgradeAction;

pub type UpdateId = u32;

/// A high‐level multisig operation that participants can propose.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub enum MultisigAction {
    /// Cancel a pending action
    Cancel(CancelAction),
    /// Execute a committed action
    Enact(EnactAction),
    /// Propose an upgrade
    Upgrade(UpgradeAction),
}
