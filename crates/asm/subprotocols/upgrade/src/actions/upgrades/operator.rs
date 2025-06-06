use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;

use crate::{crypto::PubKey, error::UpgradeError};

/// Represents a change to the Bridge Operator Set
/// * removes the specified `old_members` from the set
/// * adds the specified `new_members`
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct OperatorSetUpdate {
    new_members: Vec<PubKey>,
    old_members: Vec<PubKey>,
}

impl OperatorSetUpdate {
    pub fn new(new_members: Vec<PubKey>, old_members: Vec<PubKey>) -> Self {
        Self {
            new_members,
            old_members,
        }
    }
}

impl OperatorSetUpdate {
    pub fn extract_from_tx(_tx: &TxInput<'_>) -> Result<Self, UpgradeError> {
        // Placeholder for actual extraction logic
        let action = OperatorSetUpdate::new(vec![], vec![]);
        Ok(action)
    }
}
