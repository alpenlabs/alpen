use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;

use crate::{
    error::UpgradeError,
    txs::{ENACT_TX_TYPE, UpdateId},
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
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
    pub fn extract_from_tx(tx: &TxInput<'_>) -> Result<Self, UpgradeError> {
        // sanity check
        assert_eq!(tx.tag().tx_type(), ENACT_TX_TYPE);

        let id = 0;
        Ok(EnactAction::new(id))
    }
}
