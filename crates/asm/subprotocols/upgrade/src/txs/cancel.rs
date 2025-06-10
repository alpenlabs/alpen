use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;

use crate::{
    error::UpgradeError,
    txs::{CANCEL_TX_TYPE, UpdateId},
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct CancelAction {
    /// ID of the update that needs to be cancelled
    target_id: UpdateId,
}

impl CancelAction {
    pub fn new(id: UpdateId) -> Self {
        CancelAction { target_id: id }
    }

    pub fn target_id(&self) -> &UpdateId {
        &self.target_id
    }
}

impl CancelAction {
    /// Extracts a CancelAction from a transaction input.
    /// This is a placeholder function and should be replaced with actual logic.
    pub fn extract_from_tx(tx: &TxInput<'_>) -> Result<Self, UpgradeError> {
        // sanity check
        assert_eq!(tx.tag().tx_type(), CANCEL_TX_TYPE);

        let id = 0;
        Ok(CancelAction::new(id))
    }
}
