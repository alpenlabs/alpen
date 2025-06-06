use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;
use zkaleido::VerifyingKey;

use crate::{error::DeserializeError, roles::StrataProof};

pub const VK_UPDATE_TX_TYPE: u8 = 2;

/// Represents an update to the verifying key used for a particular Strata
/// proof layer.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct VerifyingKeyUpdate {
    new_vk: VerifyingKey,
    kind: StrataProof,
}

impl VerifyingKeyUpdate {
    pub fn new(new_vk: VerifyingKey, kind: StrataProof) -> Self {
        Self { new_vk, kind }
    }

    pub fn proof_kind(&self) -> &StrataProof {
        &self.kind
    }
}

impl VerifyingKeyUpdate {
    pub fn extract_from_tx(_tx: &TxInput<'_>) -> Result<Self, DeserializeError> {
        let action = VerifyingKeyUpdate::new(VerifyingKey::default(), StrataProof::OlStf);
        Ok(action)
    }
}
