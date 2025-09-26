use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::buf::Buf32;

/// An update to the Bridge Operator Set:
/// - removes the specified `remove_members`
/// - adds the specified `add_members`
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct OperatorSetUpdate {
    add_members: Vec<Buf32>,
    remove_members: Vec<Buf32>,
}

impl OperatorSetUpdate {
    /// Creates a new `OperatorSetUpdate`.
    pub fn new(add_members: Vec<Buf32>, remove_members: Vec<Buf32>) -> Self {
        Self {
            add_members,
            remove_members,
        }
    }

    /// Borrow the list of operator public keys to add.
    pub fn add_members(&self) -> &[Buf32] {
        &self.add_members
    }

    /// Borrow the list of operator public keys to remove.
    pub fn remove_members(&self) -> &[Buf32] {
        &self.remove_members
    }

    /// Consume and return the inner vectors `(add_members, remove_members)`.
    pub fn into_inner(self) -> (Vec<Buf32>, Vec<Buf32>) {
        (self.add_members, self.remove_members)
    }
}
