use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInputRef;

use crate::{actions::UpdateId, constants::ENACT_TX_TYPE, error::UpgradeTxParseError};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize, Arbitrary)]
pub struct EnactAction {
    target_id: UpdateId,
}

impl EnactAction {
    pub fn new(target_id: UpdateId) -> Self {
        EnactAction { target_id }
    }

    pub fn target_id(&self) -> &UpdateId {
        &self.target_id
    }
}

impl EnactAction {
    /// Extracts a CancelAction from a transaction input.
    /// This is a placeholder function and should be replaced with actual logic.
    pub fn extract_from_tx(tx: &TxInputRef<'_>) -> Result<Self, UpgradeTxParseError> {
        // sanity check
        assert_eq!(tx.tag().tx_type(), ENACT_TX_TYPE);

        let id = 0;
        Ok(EnactAction::new(id))
    }
}
