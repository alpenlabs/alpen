use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInputRef;

use crate::{actions::UpdateId, constants::CANCEL_TX_TYPE, error::AdministrationTxParseError};

#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct CancelAction {
    /// ID of the update that needs to be cancelled.
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
    /// FIXME: This is a placeholder function and should be replaced with actual logic.
    pub fn extract_from_tx(tx: &TxInputRef<'_>) -> Result<Self, AdministrationTxParseError> {
        // sanity check
        assert_eq!(tx.tag().tx_type(), CANCEL_TX_TYPE);

        let id = 0;
        Ok(CancelAction::new(id))
    }
}
