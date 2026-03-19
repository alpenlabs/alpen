//! Transaction proof verification implementation.

use strata_acct_types::{AccumulatorClaim, Mmr64, RawMerkleProof, StrataHasher};
use strata_ledger_types::{ProofVerifyError, TxProofVerifier};
use strata_merkle::MerkleProof;
use strata_ol_chain_types_new::{ProofSatisfierList, RawMerkleProofList, TxProofs};
use strata_predicate::PredicateKey;

/// Concrete implementation of [`TxProofVerifier`] that walks over the proof
/// fields in a transaction.
#[derive(Debug)]
pub struct TxProofVerifierImpl<'a> {
    predicate_satisfiers: Option<&'a ProofSatisfierList>,
    accumulator_proofs: Option<&'a RawMerkleProofList>,
    next_predicate_idx: usize,
    next_accumulator_idx: usize,
}

impl<'a> TxProofVerifierImpl<'a> {
    /// Creates a new verifier from the transaction's proof fields.
    pub fn new(tx_proofs: &'a TxProofs) -> Self {
        Self {
            predicate_satisfiers: tx_proofs.predicate_satisfiers(),
            accumulator_proofs: tx_proofs.accumulator_proofs(),
            next_predicate_idx: 0,
            next_accumulator_idx: 0,
        }
    }
}

impl TxProofVerifier for TxProofVerifierImpl<'_> {
    fn verify_next_mmr_proof(
        &mut self,
        root: &Mmr64,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError> {
        let proofs = self
            .accumulator_proofs
            .ok_or(ProofVerifyError::NoNextProof)?;

        let all_proofs = proofs.proofs();
        if self.next_accumulator_idx >= all_proofs.len() {
            return Err(ProofVerifyError::NoNextProof);
        }

        let raw_proof: &RawMerkleProof = &all_proofs[self.next_accumulator_idx];
        self.next_accumulator_idx += 1;

        // Build MerkleProof from the raw cohashes and the claim index.
        let cohashes: Vec<[u8; 32]> = raw_proof
            .cohashes
            .iter()
            .map(|h| h.0)
            .collect();

        let proof = MerkleProof::from_cohashes(cohashes, claim.idx());
        let entry_hash: [u8; 32] = claim.entry_hash().into();
        let generic_mmr = root.to_generic();

        if generic_mmr.verify::<StrataHasher>(&proof, &entry_hash) {
            Ok(())
        } else {
            Err(ProofVerifyError::InvalidProof)
        }
    }

    fn verify_next_predicate_satisfier(
        &mut self,
        key: &PredicateKey,
        claim: &[u8],
    ) -> Result<(), ProofVerifyError> {
        let satisfiers = self
            .predicate_satisfiers
            .ok_or(ProofVerifyError::NoNextProof)?;

        let all_satisfiers = satisfiers.proofs();
        if self.next_predicate_idx >= all_satisfiers.len() {
            return Err(ProofVerifyError::NoNextProof);
        }

        let satisfier = &all_satisfiers[self.next_predicate_idx];
        self.next_predicate_idx += 1;

        key.verify_claim_witness(claim, satisfier.proof())
            .map_err(|_| ProofVerifyError::InvalidProof)
    }

    fn is_exhausted(&self) -> bool {
        let pred_done = match self.predicate_satisfiers {
            Some(s) => self.next_predicate_idx >= s.proofs().len(),
            None => true,
        };
        let acc_done = match self.accumulator_proofs {
            Some(p) => self.next_accumulator_idx >= p.proofs().len(),
            None => true,
        };
        pred_done && acc_done
    }
}
