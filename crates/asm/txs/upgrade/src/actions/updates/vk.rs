use borsh::{BorshDeserialize, BorshSerialize};
use strata_asm_common::TxInput;
use zkaleido::VerifyingKey;

use crate::{error::UpgradeTxParseError, roles::ProofType};

/// An update to the verifying key for a given Strata proof layer.
#[derive(Debug, Clone, Eq, PartialEq, BorshSerialize, BorshDeserialize)]
pub struct VerifyingKeyUpdate {
    vk: VerifyingKey,
    kind: ProofType,
}

impl VerifyingKeyUpdate {
    /// Create a new `VerifyingKeyUpdate`.
    pub fn new(vk: VerifyingKey, kind: ProofType) -> Self {
        Self { vk, kind }
    }

    /// Borrow the updated verifying key.
    pub fn vk(&self) -> &VerifyingKey {
        &self.vk
    }

    /// Get the associated proof kind.
    pub fn kind(&self) -> ProofType {
        self.kind
    }

    /// Consume and return the inner values.
    pub fn into_inner(self) -> (VerifyingKey, ProofType) {
        (self.vk, self.kind)
    }

    /// Extract a `VerifyingKeyUpdate` from a transaction input.
    ///
    /// Placeholder logic: replace with actual deserialization.
    pub fn extract_from_tx(_tx: &TxInput<'_>) -> Result<Self, UpgradeTxParseError> {
        // TODO: parse `TxInput` to obtain vk bytes and proof kind
        Ok(Self::new(VerifyingKey::default(), ProofType::OlStf))
    }
}

/// Allow borrowing the inner verifying key via `AsRef`.
impl AsRef<VerifyingKey> for VerifyingKeyUpdate {
    fn as_ref(&self) -> &VerifyingKey {
        &self.vk
    }
}
