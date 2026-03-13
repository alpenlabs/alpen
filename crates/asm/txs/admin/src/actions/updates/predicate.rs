use arbitrary::Arbitrary;
use ssz::{Decode, DecodeError, Encode};
use ssz_derive::{Decode as DeriveDecode, Encode as DeriveEncode};
use strata_predicate::PredicateKey;

use crate::{actions::Sighash, constants::AdminTxType};

/// An update to the verifying key for a given Strata proof layer.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, DeriveEncode, DeriveDecode)]
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

#[derive(Copy, Clone, Debug, Eq, PartialEq, Arbitrary)]
pub enum ProofType {
    Asm,
    OLStf,
}

impl Encode for ProofType {
    fn is_ssz_fixed_len() -> bool {
        <u8 as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <u8 as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        let value = match self {
            Self::Asm => 0u8,
            Self::OLStf => 1u8,
        };
        value.ssz_append(buf);
    }

    fn ssz_bytes_len(&self) -> usize {
        <u8 as Encode>::ssz_fixed_len()
    }
}

impl Decode for ProofType {
    fn is_ssz_fixed_len() -> bool {
        <u8 as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <u8 as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        match u8::from_ssz_bytes(bytes)? {
            0 => Ok(Self::Asm),
            1 => Ok(Self::OLStf),
            value => Err(DecodeError::BytesInvalid(format!(
                "invalid proof type discriminant: {value}"
            ))),
        }
    }
}
