use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;

use crate::{crypto::PubKey, error::UpgradeError};

/// An update to the Bridge Operator Set:
/// - removes the specified `old_members`
/// - adds the specified `new_members`
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct OperatorSetUpdate {
    new_members: Vec<PubKey>,
    old_members: Vec<PubKey>,
}

impl OperatorSetUpdate {
    /// Creates a new `OperatorSetUpdate`.
    pub fn new(new_members: Vec<PubKey>, old_members: Vec<PubKey>) -> Self {
        Self {
            new_members,
            old_members,
        }
    }

    /// Borrow the list of new operator public keys to add.
    pub fn new_members(&self) -> &[PubKey] {
        &self.new_members
    }

    /// Borrow the list of old operator public keys to remove.
    pub fn old_members(&self) -> &[PubKey] {
        &self.old_members
    }

    /// Consume and return the inner vectors `(new_members, old_members)`.
    pub fn into_inner(self) -> (Vec<PubKey>, Vec<PubKey>) {
        (self.new_members, self.old_members)
    }

    /// Extracts an `OperatorSetUpdate` from a transaction input.
    ///
    /// Placeholder logic: replace with real parsing implementation.
    pub fn extract_from_tx(_tx: &TxInput<'_>) -> Result<Self, UpgradeError> {
        // TODO: parse `_tx` to determine which keys to add/remove
        Ok(Self::new(Vec::new(), Vec::new()))
    }
}
