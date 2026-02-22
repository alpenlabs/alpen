use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_predicate::PredicateKey;

use crate::{actions::Sighash, constants::AdminTxType};

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

impl Sighash for PredicateUpdate {
    fn tx_type(&self) -> AdminTxType {
        match self.kind {
            ProofType::Asm => AdminTxType::AsmStfVkUpdate,
            ProofType::OLStf => AdminTxType::OlStfVkUpdate,
        }
    }

    /// Returns the raw bytes of the [`PredicateKey`].
    ///
    /// Only the key is included because the proof kind is already covered by
    /// the [`AdminTxType`] returned from [`tx_type`](Self::tx_type).
    fn sighash_payload(&self) -> Vec<u8> {
        self.key.as_buf_ref().to_bytes()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Arbitrary, BorshDeserialize, BorshSerialize)]
pub enum ProofType {
    Asm,
    OLStf,
}
