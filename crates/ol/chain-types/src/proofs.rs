//! Proof-related types for OL chain.

use strata_acct_types::{AccumulatorClaim, RawMerkleProof};

use crate::ssz_generated::ssz::proofs::*;

impl ClaimList {
    /// Creates a new claim list from the given claims.
    ///
    /// Returns `None` if the number of claims exceeds the SSZ list maximum.
    pub fn new(claims: Vec<AccumulatorClaim>) -> Option<Self> {
        Some(Self {
            claims: claims.try_into().ok()?,
        })
    }

    pub fn claims(&self) -> &[AccumulatorClaim] {
        &self.claims
    }
}

impl RawMerkleProofList {
    pub fn proofs(&self) -> &[RawMerkleProof] {
        &self.proofs
    }
}

impl ProofSatisfier {
    pub fn proof(&self) -> &[u8] {
        &self.proof
    }
}

impl ProofSatisfierList {
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
            claims: claims.into(),
        })
    }

    fn raw_merkle_proof_strategy() -> impl Strategy<Value = RawMerkleProof> {
        prop::collection::vec(any::<[u8; 32]>(), 0..10).prop_map(|hashes| RawMerkleProof {
            cohashes: hashes
                .into_iter()
                .map(|h| h.into())
                .collect::<Vec<_>>()
                .into(),
        })
    }

    fn raw_merkle_proof_list_strategy() -> impl Strategy<Value = RawMerkleProofList> {
        prop::collection::vec(raw_merkle_proof_strategy(), 0..10).prop_map(|proofs| {
            RawMerkleProofList {
                proofs: proofs.into(),
            }
        })
    }

    fn proof_satisfier_strategy() -> impl Strategy<Value = ProofSatisfier> {
        prop::collection::vec(any::<u8>(), 0..256).prop_map(|proof| ProofSatisfier {
            proof: proof.into(),
        })
    }

    fn proof_satisfier_list_strategy() -> impl Strategy<Value = ProofSatisfierList> {
        prop::collection::vec(proof_satisfier_strategy(), 0..10).prop_map(|proofs| {
            ProofSatisfierList {
                proofs: proofs.into(),
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
