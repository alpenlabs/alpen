use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;
use strata_primitives::buf::Buf32;

use crate::{
    error::UpgradeError,
    txs::{CANCEL_TX_TYPE, updates::id::UpdateId},
};

#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, BorshSerialize, BorshDeserialize)]
pub struct CancelAction {
    id: UpdateId,
}

impl CancelAction {
    pub fn new(id: UpdateId) -> Self {
        CancelAction { id }
    }

    pub fn id(&self) -> &UpdateId {
        &self.id
    }
}

impl CancelAction {
    /// Extracts a CancelAction from a transaction input.
    /// This is a placeholder function and should be replaced with actual logic.
    pub fn extract_from_tx(tx: &TxInput<'_>) -> Result<Self, UpgradeError> {
        // sanity check
        assert_eq!(tx.tag().tx_type(), CANCEL_TX_TYPE);

        let id = Buf32::zero().into();
        Ok(CancelAction::new(id))
    }
}
