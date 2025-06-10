use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;
use strata_primitives::buf::Buf32;

use crate::error::UpgradeTxParseError;

/// An update to the public key of the sequencer
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct SequencerUpdate {
    pub_key: Buf32,
}

impl SequencerUpdate {
    /// Create a new `SequencerUpdate` from the given public key.
    pub fn new(pub_key: Buf32) -> Self {
        Self { pub_key }
    }

    /// Borrow the new sequencer public key.
    pub fn pub_key(&self) -> &Buf32 {
        &self.pub_key
    }

    /// Consume and return the inner public key.
    pub fn into_inner(self) -> Buf32 {
        self.pub_key
    }

    /// Extract a `SequencerUpdate` from a transaction input.
    ///
    /// Placeholder: replace with real parsing logic.
    pub fn extract_from_tx(_tx: &TxInput<'_>) -> Result<Self, UpgradeTxParseError> {
        // TODO: parse TxInput to obtain new sequencer Buf32
        Ok(Self::new(Buf32::default()))
    }
}
