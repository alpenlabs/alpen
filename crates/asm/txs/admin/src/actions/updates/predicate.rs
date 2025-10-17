use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_predicate::PredicateKey;
use strata_primitives::roles::ProofType;

/// An update to the verifying key for a given Strata proof layer.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub struct PredicateUpdate {
    key: PredicateKey,
    kind: ProofType,
}

impl PredicateUpdate {
    /// Create a new `VerifyingKeyUpdate`.
    pub fn new(key: PredicateKey, kind: ProofType) -> Self {
        Self { key, kind }
    }

    /// Borrow the updated verifying key.
    pub fn key(&self) -> &PredicateKey {
        &self.key
    }

    /// Get the associated proof kind.
    pub fn kind(&self) -> ProofType {
        self.kind
    }

    /// Consume and return the inner values.
    pub fn into_inner(self) -> (PredicateKey, ProofType) {
        (self.key, self.kind)
    }
}
