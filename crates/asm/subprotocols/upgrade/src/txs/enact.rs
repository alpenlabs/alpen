use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;
use strata_primitives::buf::Buf32;

use crate::{
    error::UpgradeError,
    txs::{ENACT_TX_TYPE, updates::id::UpdateId},
};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct EnactAction {
    id: UpdateId,
}

impl EnactAction {
    pub fn new(id: UpdateId) -> Self {
        EnactAction { id }
    }

    pub fn id(&self) -> &UpdateId {
        &self.id
    }
}

impl EnactAction {
    /// Extracts a CancelAction from a transaction input.
    /// This is a placeholder function and should be replaced with actual logic.
    pub fn extract_from_tx(tx: &TxInput<'_>) -> Result<Self, UpgradeError> {
        // sanity check
        assert_eq!(tx.tag().tx_type(), ENACT_TX_TYPE);

        let id = Buf32::zero().into();
        Ok(EnactAction::new(id))
    }
}
