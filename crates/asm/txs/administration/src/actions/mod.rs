use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};

mod cancel;
pub mod updates;

pub use cancel::CancelAction;
pub use updates::UpdateAction;

pub type UpdateId = u32;

/// A high‐level multisig operation that participants can propose.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub enum MultisigAction {
    /// Cancel a pending action
    Cancel(CancelAction),
    /// Propose an update
    Update(UpdateAction),
}
