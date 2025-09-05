use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInputRef;
use strata_primitives::buf::Buf32;

use crate::error::AdministrationTxParseError;

/// An update to the Bridge Operator Set:
/// - removes the specified `old_members`
/// - adds the specified `new_members`
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct OperatorSetUpdate {
    new_members: Vec<Buf32>,
    old_members: Vec<Buf32>,
}

impl OperatorSetUpdate {
    /// Creates a new `OperatorSetUpdate`.
    pub fn new(new_members: Vec<Buf32>, old_members: Vec<Buf32>) -> Self {
        Self {
            new_members,
            old_members,
        }
    }

    /// Borrow the list of new operator public keys to add.
    pub fn new_members(&self) -> &[Buf32] {
        &self.new_members
    }

    /// Borrow the list of old operator public keys to remove.
    pub fn old_members(&self) -> &[Buf32] {
        &self.old_members
    }

    /// Consume and return the inner vectors `(new_members, old_members)`.
    pub fn into_inner(self) -> (Vec<Buf32>, Vec<Buf32>) {
        (self.new_members, self.old_members)
    }

    /// Extracts an `OperatorSetUpdate` from a transaction input.
    ///
    /// Placeholder logic: replace with real parsing implementation.
    pub fn extract_from_tx(_tx: &TxInputRef<'_>) -> Result<Self, AdministrationTxParseError> {
        // TODO: parse `_tx` to determine which keys to add/remove
        Ok(Self::new(Vec::new(), Vec::new()))
    }
}
