//! Transaction proof verification implementation.
#![expect(missing_debug_implementations, reason = "opaque hash types")]

use strata_acct_types::{AccumulatorClaim, Mmr64, RawMerkleProof};
use strata_merkle::{MerkleProofB32, Mmr64B32, MmrState, ProofData, Sha256Hasher, compute_root};
use strata_ol_state_types::{
    AccountTypeStateRef, IAccountState, ISnarkAccountState, IStateAccessor, ProofVerifyError,
    TxProofVerifier,
};
use strata_ol_tx_types_v1::{ProofSatisfierListV1, RawMerkleProofListV1, TxProofsV1};
use strata_predicate::PredicateKey;
use tracing::warn;

use crate::rules::RuleAmendments;

/// Context for accumulators/keys we verify proofs against when verifying a
/// proof.
pub(crate) struct TxProofVerificationContext<'a> {
    l1_block_refs_mmr: &'a Mmr64,
    local_inbox_mmr: Option<&'a Mmr64B32>,
    local_predicate_key: Option<&'a PredicateKey>,
}

impl<'a> TxProofVerificationContext<'a> {
    pub(crate) fn from_account_and_state<S: IStateAccessor>(
        state: &'a S,
        account: &'a S::AccountState,
    ) -> Self {
        let l1_block_refs_mmr = state.l1_block_refs_mmr();

        let (local_inbox_mmr, local_predicate_key) = match account.type_state() {
            AccountTypeStateRef::Empty => (None, None),
            AccountTypeStateRef::Snark(tstate) => {
                (Some(tstate.inbox_mmr()), Some(tstate.update_vk()))
            }
        };

        Self {
            l1_block_refs_mmr,
            local_inbox_mmr,
            local_predicate_key,
        }
    }
}

/// Tracks context for verifying the various proofs from a tx.
pub(crate) struct TxProofsTracker<'a> {
    accumulator_proofs: Option<&'a RawMerkleProofListV1>,
    next_acc_proof_idx: usize,
    predicate_satisfiers: Option<&'a ProofSatisfierListV1>,
    next_pred_proof_idx: usize,
}

impl<'a> TxProofsTracker<'a> {
    pub(crate) fn from_txproofs(tx_proofs: &'a TxProofsV1) -> Self {
        Self {
            predicate_satisfiers: tx_proofs.predicate_satisfiers(),
            accumulator_proofs: tx_proofs.accumulator_proofs(),
            next_pred_proof_idx: 0,
            next_acc_proof_idx: 0,
        }
    }

    fn acc_proofs_cnt(&self) -> usize {
        self.accumulator_proofs
            .map(|l| l.proofs().len())
            .unwrap_or_default()
    }

    fn next_acc_proof(&self) -> Option<&RawMerkleProof> {
        let acc_proofs = self.accumulator_proofs?;
        acc_proofs.proofs().get(self.next_acc_proof_idx)
    }

    fn inc_next_acc_proof(&mut self) -> Result<(), ProofVerifyError> {
        let cnt = self.acc_proofs_cnt();
        if self.next_acc_proof_idx == cnt {
            return Err(ProofVerifyError::NoNextProof);
        }

        self.next_acc_proof_idx += 1;
        Ok(())
    }

    fn is_acc_proofs_done(&self) -> bool {
        self.next_acc_proof_idx == self.acc_proofs_cnt()
    }

    fn pred_proofs_cnt(&self) -> usize {
        self.predicate_satisfiers
            .map(|l| l.proofs().len())
            .unwrap_or_default()
    }

    fn next_pred_proof(&self) -> Option<&[u8]> {
        let pred_proofs = self.predicate_satisfiers?;
        pred_proofs
            .proofs()
            .get(self.next_pred_proof_idx)
            .map(|e| e.proof())
    }

    fn inc_next_pred_proof(&mut self) -> Result<(), ProofVerifyError> {
        let cnt = self.pred_proofs_cnt();
        if self.next_pred_proof_idx == cnt {
            return Err(ProofVerifyError::NoNextProof);
        }

        self.next_pred_proof_idx += 1;
        Ok(())
    }

    fn is_pred_proofs_done(&self) -> bool {
        self.next_pred_proof_idx == self.pred_proofs_cnt()
    }

    fn is_all_done(&self) -> bool {
        self.is_acc_proofs_done() && self.is_pred_proofs_done()
    }
}

/// Concrete implementation of [`TxProofVerifier`] that walks over the proof
/// fields in a transaction.
pub struct TxProofVerifierImpl<'a> {
    state_ctx: TxProofVerificationContext<'a>,
    proof_tracker: TxProofsTracker<'a>,
    amendments: RuleAmendments,
}

impl<'a> TxProofVerifierImpl<'a> {
    /// Creates a new verifier from the account state context and proof
    /// tracker, which verifies accumulator proofs as `amendments` select.
    pub(crate) fn new(
        state_ctx: TxProofVerificationContext<'a>,
        proof_tracker: TxProofsTracker<'a>,
        amendments: RuleAmendments,
    ) -> Self {
        Self {
            state_ctx,
            proof_tracker,
            amendments,
        }
    }

    /// Pops the next accumulator proof from the tracker and verifies it against
    /// the provided MMR and claim.
    fn verify_next_mmr_proof(
        &mut self,
        mmr: &Mmr64B32,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError> {
        let raw_proof = self
            .proof_tracker
            .next_acc_proof()
            .ok_or(ProofVerifyError::NoNextProof)?
            .clone();

        let indexed_proof = raw_proof.into_indexed(claim.idx());
        let leaf: [u8; 32] = claim.entry_hash().0;

        let verified = if self.amendments.accumulator_proofs_bind_index() {
            mmr.verify(&indexed_proof, &leaf)
        } else {
            verify_mmr_proof_by_path_len(mmr, &indexed_proof, &leaf)
        };
        if !verified {
            return Err(ProofVerifyError::InvalidProof);
        }

        self.proof_tracker.inc_next_acc_proof()?;
        Ok(())
    }
}

/// Verifies an MMR proof the way `strata-merkle` did before v0.5.0, which the
/// rules without the index binding amendment are fixed to.
///
/// The number of cohashes selects the peak, and the path from `leaf` must
/// hash to it. Each cohash reads one bit of the proof's index, from the
/// lowest, to pick its side, and the bits above the path are ignored. So the
/// index is checked only modulo the size of the peak: an entry also verifies
/// under the index of an entry at the same position in another peak, or under
/// an index the MMR does not hold yet. [`Mmr64B32::verify`] rejects both.
///
/// A proof holds at most 64 cohashes. With all 64 the earlier check compared
/// against a peak that could only match through a SHA-256 preimage, so this
/// rejects such a proof outright.
fn verify_mmr_proof_by_path_len(mmr: &Mmr64B32, proof: &MerkleProofB32, leaf: &[u8; 32]) -> bool {
    let Some(height) = u8::try_from(proof.cohashes_len())
        .ok()
        .filter(|height| u32::from(*height) < u64::BITS)
    else {
        return false;
    };
    let Some(peak) = mmr.get_peak(height) else {
        return false;
    };
    compute_root::<_, Sha256Hasher>(proof, leaf) == *peak
}

impl TxProofVerifier for TxProofVerifierImpl<'_> {
    fn verify_inbox_mmr_proof_next(
        &mut self,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError> {
        let inbox_mmr = self
            .state_ctx
            .local_inbox_mmr
            .ok_or(ProofVerifyError::InvalidContext)?;

        self.verify_next_mmr_proof(inbox_mmr, claim)
    }

    fn verify_l1_block_ref_mmr_proof_next(
        &mut self,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError> {
        self.verify_next_mmr_proof(self.state_ctx.l1_block_refs_mmr, claim)
    }

    fn verify_local_predicate_next(&mut self, claim: &[u8]) -> Result<(), ProofVerifyError> {
        let predicate_key = self
            .state_ctx
            .local_predicate_key
            .ok_or(ProofVerifyError::InvalidContext)?;

        let witness = self
            .proof_tracker
            .next_pred_proof()
            .ok_or(ProofVerifyError::NoNextProof)?;

        predicate_key
            .verify_claim_witness(claim, witness)
            .map_err(|e| {
                warn!(
                    error = %e,
                    claim_len = claim.len(),
                    witness_len = witness.len(),
                    "predicate verification failed"
                );
                ProofVerifyError::InvalidProof
            })?;

        self.proof_tracker.inc_next_pred_proof()?;
        Ok(())
    }

    fn is_exhausted(&self) -> bool {
        self.proof_tracker.is_all_done()
    }
}

#[cfg(test)]
mod tests {
    use ssz_primitives::FixedBytes;
    use strata_acct_types::{RawMerkleProof, StrataHasher};
    use strata_merkle::{CompactMmr64, MerkleProof, Mmr};
    use strata_ol_tx_types_v1::{
        ProofSatisfierListV1, ProofSatisfierV1, RawMerkleProofListV1, TxProofsV1,
    };
    use strata_predicate::{PredicateError, PredicateKey, PredicateTypeId};

    use super::*;

    /// Builds an MMR over six leaves, whose peaks hold the entries 0 to 3 and
    /// 4 to 5, and returns it with the leaves and the current proof of each.
    fn six_leaf_mmr() -> (Mmr64, Vec<[u8; 32]>, Vec<RawMerkleProof>) {
        let leaves: Vec<[u8; 32]> = (1..=6).map(|byte| [byte; 32]).collect();
        let mut mmr = Mmr64::from_generic(&CompactMmr64::new(64));
        let mut proofs: Vec<MerkleProof<[u8; 32]>> = Vec::new();
        for leaf in &leaves {
            let proof =
                Mmr::<StrataHasher>::add_leaf_updating_proof_list(&mut mmr, *leaf, &mut proofs)
                    .expect("the MMR has room for the leaf");
            proofs.push(proof);
        }
        let raw_proofs = proofs
            .iter()
            .map(|proof| raw_proof(proof.cohashes()))
            .collect();
        (mmr, leaves, raw_proofs)
    }

    fn raw_proof(cohashes: &[[u8; 32]]) -> RawMerkleProof {
        RawMerkleProof {
            cohashes: cohashes
                .iter()
                .map(|hash| FixedBytes::from(*hash))
                .collect::<Vec<_>>()
                .try_into()
                .expect("cohashes fit the proof depth"),
        }
    }

    /// Verifies one L1 block ref claim against `mmr` with `proof`.
    fn verify_claim(
        mmr: &Mmr64,
        proof: &RawMerkleProof,
        claim: &AccumulatorClaim,
        amendments: RuleAmendments,
    ) -> Result<(), ProofVerifyError> {
        let tx_proofs = TxProofsV1::new(
            None,
            Some(RawMerkleProofListV1 {
                proofs: vec![proof.clone()]
                    .try_into()
                    .expect("one proof fits the list"),
            }),
        );
        let state_ctx = TxProofVerificationContext {
            l1_block_refs_mmr: mmr,
            local_inbox_mmr: None,
            local_predicate_key: None,
        };
        let mut verifier = TxProofVerifierImpl::new(
            state_ctx,
            TxProofsTracker::from_txproofs(&tx_proofs),
            amendments,
        );
        verifier.verify_l1_block_ref_mmr_proof_next(claim)
    }

    #[test]
    fn test_accumulator_proof_of_the_claimed_entry_verifies_under_both_rules() {
        let (mmr, leaves, proofs) = six_leaf_mmr();
        for (index, (leaf, proof)) in leaves.iter().zip(&proofs).enumerate() {
            let claim = AccumulatorClaim::new(index as u64, *leaf);
            for amendments in [RuleAmendments::NONE, RuleAmendments::V3] {
                verify_claim(&mmr, proof, &claim, amendments)
                    .unwrap_or_else(|err| panic!("entry {index} under {amendments:?}: {err}"));
            }
        }
    }

    #[test]
    fn test_accumulator_proof_binds_the_index_only_with_the_amendment() {
        let (mmr, leaves, proofs) = six_leaf_mmr();

        // Entry 0 sits at position 0 of the four-entry peak. Index 4 is
        // position 0 of the other peak, and index 8 is one the MMR does not
        // hold yet. Both share index 0's two low bits, which are all the path
        // of two cohashes reads.
        for index in [4, 8] {
            let claim = AccumulatorClaim::new(index, leaves[0]);
            verify_claim(&mmr, &proofs[0], &claim, RuleAmendments::NONE)
                .unwrap_or_else(|err| panic!("index {index} without the amendment: {err}"));
            let err = verify_claim(&mmr, &proofs[0], &claim, RuleAmendments::V3)
                .expect_err("the amended rules bind the index");
            assert!(matches!(err, ProofVerifyError::InvalidProof), "{err}");
        }
    }

    #[test]
    fn test_accumulator_proof_with_a_wrong_path_fails_under_both_rules() {
        let (mmr, leaves, proofs) = six_leaf_mmr();

        // Index 1 differs from entry 0's index in a bit the path reads, a
        // wrong leaf hashes to another root, and no peak has the height of a
        // 64-cohash path.
        let wrong_position = (AccumulatorClaim::new(1, leaves[0]), proofs[0].clone());
        let wrong_leaf = (AccumulatorClaim::new(0, leaves[1]), proofs[0].clone());
        let overlong = (
            AccumulatorClaim::new(0, leaves[0]),
            raw_proof(&[[0; 32]; 64]),
        );
        for (claim, proof) in [wrong_position, wrong_leaf, overlong] {
            for amendments in [RuleAmendments::NONE, RuleAmendments::V3] {
                let err = verify_claim(&mmr, &proof, &claim, amendments)
                    .expect_err("the proof does not lead to a peak");
                assert!(matches!(err, ProofVerifyError::InvalidProof), "{err}");
            }
        }
    }

    fn make_acc_proofs(n: usize) -> TxProofsV1 {
        let proofs: Vec<RawMerkleProof> = (0..n).map(|_| RawMerkleProof::new_zero()).collect();
        TxProofsV1::new(
            None,
            Some(RawMerkleProofListV1 {
                proofs: proofs
                    .try_into()
                    .expect("proofs should not exceed capacity"),
            }),
        )
    }

    fn make_pred_proofs(n: usize) -> TxProofsV1 {
        let proofs: Vec<ProofSatisfierV1> = (0..n)
            .map(|i| ProofSatisfierV1 {
                proof: vec![i as u8]
                    .try_into()
                    .expect("proof should not exceed capacity"),
            })
            .collect();
        TxProofsV1::new(
            Some(ProofSatisfierListV1 {
                proofs: proofs
                    .try_into()
                    .expect("proofs should not exceed capacity"),
            }),
            None,
        )
    }

    #[test]
    fn test_acc_proof_bookkeeping() {
        let tx_proofs = make_acc_proofs(2);
        let mut tracker = TxProofsTracker::from_txproofs(&tx_proofs);

        assert!(!tracker.is_acc_proofs_done());
        assert!(tracker.next_acc_proof().is_some());

        tracker
            .inc_next_acc_proof()
            .expect("first inc should succeed");
        assert!(!tracker.is_acc_proofs_done());

        tracker
            .inc_next_acc_proof()
            .expect("second inc should succeed");
        assert!(tracker.is_acc_proofs_done());
        assert!(tracker.next_acc_proof().is_none());

        // Incrementing past the end returns NoNextProof.
        let err = tracker.inc_next_acc_proof().unwrap_err();
        assert!(matches!(err, ProofVerifyError::NoNextProof));
    }

    #[test]
    fn test_pred_proof_bookkeeping() {
        let tx_proofs = make_pred_proofs(2);
        let mut tracker = TxProofsTracker::from_txproofs(&tx_proofs);

        assert!(!tracker.is_pred_proofs_done());
        assert!(tracker.next_pred_proof().is_some());

        tracker
            .inc_next_pred_proof()
            .expect("first inc should succeed");
        assert!(!tracker.is_pred_proofs_done());

        tracker
            .inc_next_pred_proof()
            .expect("second inc should succeed");
        assert!(tracker.is_pred_proofs_done());
        assert!(tracker.next_pred_proof().is_none());

        // Incrementing past the end returns NoNextProof.
        let err = tracker.inc_next_pred_proof().unwrap_err();
        assert!(matches!(err, ProofVerifyError::NoNextProof));
    }

    #[test]
    fn sp1_groth16_predicate_verifier_is_available() {
        let predicate_key = PredicateKey::try_new(PredicateTypeId::Sp1Groth16, Vec::new())
            .expect("predicate condition must fit within the maximum length");
        let err = predicate_key
            .verify_claim_witness(&[], &[])
            .expect_err("empty condition is invalid, but verifier must be compiled in");

        assert!(
            !matches!(
                err,
                PredicateError::UnsupportedPredicateType {
                    id: PredicateTypeId::Sp1Groth16,
                    ..
                }
            ),
            "Sp1Groth16 verifier feature must be enabled for OL STF replay"
        );
    }

    #[test]
    fn test_is_all_done_requires_consuming_all_proofs() {
        let acc_proofs: Vec<RawMerkleProof> = (0..2).map(|_| RawMerkleProof::new_zero()).collect();
        let pred_proofs: Vec<ProofSatisfierV1> = (0..2)
            .map(|i| ProofSatisfierV1 {
                proof: vec![i as u8]
                    .try_into()
                    .expect("proof should not exceed capacity"),
            })
            .collect();

        let tx_proofs = TxProofsV1::new(
            Some(ProofSatisfierListV1 {
                proofs: pred_proofs
                    .try_into()
                    .expect("proofs should not exceed capacity"),
            }),
            Some(RawMerkleProofListV1 {
                proofs: acc_proofs
                    .try_into()
                    .expect("proofs should not exceed capacity"),
            }),
        );
        let mut tracker = TxProofsTracker::from_txproofs(&tx_proofs);

        // Consume one from each stream and ensure exhaustion is still false.
        tracker
            .inc_next_acc_proof()
            .expect("first acc inc should work");
        tracker
            .inc_next_pred_proof()
            .expect("first pred inc should work");
        assert!(!tracker.is_all_done(), "one proof in each stream remains");

        // Consume remaining proofs and ensure tracker now reports exhausted.
        tracker
            .inc_next_acc_proof()
            .expect("second acc inc should work");
        tracker
            .inc_next_pred_proof()
            .expect("second pred inc should work");
        assert!(tracker.is_all_done(), "all proofs should be consumed");
    }
}
