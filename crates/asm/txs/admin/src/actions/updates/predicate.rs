use arbitrary::{Arbitrary, Unstructured};
use strata_predicate::PredicateKey;

pub use crate::{PredicateUpdate, ProofType};
use crate::{actions::Sighash, constants::AdminTxType};

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
        match self.kind.value {
            0 => AdminTxType::AsmStfVkUpdate,
            1 => AdminTxType::OlStfVkUpdate,
            _ => unreachable!("invalid proof type selector {}", self.kind.value),
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

impl ProofType {
    #[expect(
        non_upper_case_globals,
        reason = "preserve the existing ProofType::Variant API"
    )]
    pub const Asm: Self = Self { value: 0 };

    #[expect(
        non_upper_case_globals,
        reason = "preserve the existing ProofType::Variant API"
    )]
    pub const OLStf: Self = Self { value: 1 };
}

impl<'a> Arbitrary<'a> for ProofType {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        if bool::arbitrary(u)? {
            Ok(Self::Asm)
        } else {
            Ok(Self::OLStf)
        }
    }
}

impl<'a> Arbitrary<'a> for PredicateUpdate {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::new(
            PredicateKey::arbitrary(u)?,
            ProofType::arbitrary(u)?,
        ))
    }
}
