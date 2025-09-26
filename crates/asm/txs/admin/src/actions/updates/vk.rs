use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_primitives::roles::ProofType;
use zkaleido::VerifyingKey;

/// An update to the verifying key for a given Strata proof layer.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
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
}

/// Allow borrowing the inner verifying key via `AsRef`.
impl AsRef<VerifyingKey> for VerifyingKeyUpdate {
    fn as_ref(&self) -> &VerifyingKey {
        &self.vk
    }
}
