//! Proof-related types for OL chain.

use ssz_types::VariableList;
use strata_acct_types::{AccumulatorClaim, RawMerkleProof};

use crate::ssz_generated::ssz::proofs::*;

impl ClaimList {
    /// Creates a new claim list from the given claims.
    ///
    /// Returns `None` if the number of claims exceeds the SSZ list maximum.
    pub fn new(claims: Vec<AccumulatorClaim>) -> Option<Self> {
        Some(Self {
            claims: VariableList::new(claims).ok()?,
        })
    }

    pub fn claims(&self) -> &[AccumulatorClaim] {
        &self.claims
    }
}

impl RawMerkleProofList {
    /// Constructs a new instance if the vec is in bounds.
    pub fn from_vec(buf: Vec<RawMerkleProof>) -> Option<Self> {
        VariableList::new(buf).ok().map(|proofs| Self { proofs })
    }

    /// Constructs from a vec, returning `None` if the vec is empty (or out of
    /// bounds).
    pub fn from_vec_nonempty(buf: Vec<RawMerkleProof>) -> Option<Self> {
        if buf.is_empty() {
            return None;
        }
        Self::from_vec(buf)
    }

    pub fn proofs(&self) -> &[RawMerkleProof] {
        &self.proofs
    }
}

impl ProofSatisfier {
    /// Constructs a new instance if the vec is in bounds.
    pub fn from_vec(buf: Vec<u8>) -> Option<Self> {
        VariableList::new(buf).ok().map(|proof| Self { proof })
    }

    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}

impl ProofSatisfierList {
    /// Constructs a new instance if the vec is in bounds.
    pub fn from_proofs(buf: Vec<ProofSatisfier>) -> Option<Self> {
        VariableList::new(buf).ok().map(|proofs| Self { proofs })
    }

    /// Wraps a single proof satisfier into a list.
    pub fn single(proof_bytes: Vec<u8>) -> Option<Self> {
        let satisfier = ProofSatisfier::from_vec(proof_bytes)?;
        Self::from_proofs(vec![satisfier])
    }

    pub fn proofs(&self) -> &[ProofSatisfier] {
        &self.proofs
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_acct_types::RawMerkleProof;
    use strata_test_utils_ssz::ssz_proptest;

    use super::*;
    use crate::test_utils::accumulator_claim_strategy;

    fn claim_list_strategy() -> impl Strategy<Value = ClaimList> {
        prop::collection::vec(accumulator_claim_strategy(), 0..10).prop_map(|claims| ClaimList {
            claims: claims
                .try_into()
                .expect("claims must fit within SSZ max length"),
        })
    }

    fn raw_merkle_proof_strategy() -> impl Strategy<Value = RawMerkleProof> {
        prop::collection::vec(any::<[u8; 32]>(), 0..10).prop_map(|hashes| RawMerkleProof {
            cohashes: hashes
                .into_iter()
                .map(|h| h.into())
                .collect::<Vec<_>>()
                .try_into()
                .expect("cohashes must fit within SSZ max length"),
        })
    }

    fn raw_merkle_proof_list_strategy() -> impl Strategy<Value = RawMerkleProofList> {
        prop::collection::vec(raw_merkle_proof_strategy(), 0..10).prop_map(|proofs| {
            RawMerkleProofList {
                proofs: proofs
                    .try_into()
                    .expect("proofs must fit within SSZ max length"),
            }
        })
    }

    fn proof_satisfier_strategy() -> impl Strategy<Value = ProofSatisfier> {
        prop::collection::vec(any::<u8>(), 0..256).prop_map(|proof| ProofSatisfier {
            proof: proof
                .try_into()
                .expect("proof bytes must fit within SSZ max length"),
        })
    }

    fn proof_satisfier_list_strategy() -> impl Strategy<Value = ProofSatisfierList> {
        prop::collection::vec(proof_satisfier_strategy(), 0..10).prop_map(|proofs| {
            ProofSatisfierList {
                proofs: proofs
                    .try_into()
                    .expect("proof satisfiers must fit within SSZ max length"),
            }
        })
    }

    mod claim_list {
        use super::*;

        ssz_proptest!(ClaimList, claim_list_strategy());
    }

    mod raw_merkle_proof_list {
        use super::*;

        ssz_proptest!(RawMerkleProofList, raw_merkle_proof_list_strategy());
    }

    mod proof_satisfier {
        use super::*;

        ssz_proptest!(ProofSatisfier, proof_satisfier_strategy());
    }

    mod proof_satisfier_list {
        use super::*;

        ssz_proptest!(ProofSatisfierList, proof_satisfier_list_strategy());
    }
}
