use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;

use crate::{crypto::PubKey, error::UpgradeError};

#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct SequencerUpdate {
    new_sequencer_pub_key: PubKey,
}

impl SequencerUpdate {
    pub fn new(new_sequencer_pub_key: PubKey) -> Self {
        Self {
            new_sequencer_pub_key,
        }
    }
}

impl SequencerUpdate {
    pub fn extract_from_tx(_tx: &TxInput<'_>) -> Result<Self, UpgradeError> {
        // Placeholder for actual extraction logic
        let action = SequencerUpdate::new(PubKey::default());
        Ok(action)
    }
}
