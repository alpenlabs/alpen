//! Transaction proof verification implementation.

use strata_acct_types::{AccumulatorClaim, Mmr64, RawMerkleProof, StrataHasher};
use strata_ledger_types::{
    AccountTypeStateRef, IAccountState, ISnarkAccountState, IStateAccessor, ProofVerifyError,
    TxProofVerifier,
};
use strata_merkle::{MerkleProof, Mmr64B32};
use strata_ol_chain_types_new::{ProofSatisfierList, RawMerkleProofList, TxProofs};
use strata_predicate::PredicateKey;

/// Context for accumulators/keys we verify proofs against when verifying a
/// proof.
#[derive(Debug)]
pub(crate) struct TxProofVerificationContext<'a> {
    asm_history_mmr: &'a Mmr64,
    local_inbox_mmr: Option<&'a Mmr64B32>,
    local_predicate_key: Option<&'a PredicateKey>,
}

impl<'a> TxProofVerificationContext<'a> {
    pub(crate) fn from_account_and_state<S: IStateAccessor>(
        state: &S,
        account: &S::AccountState,
    ) -> Self {
        let asm_history_mmr = state.asm_manifests_mmr();

        let (local_inbox_mmr, local_predicate_key) = match account.type_state() {
            AccountTypeStateRef::Empty => (None, None),
            AccountTypeStateRef::Snark(tstate) => {
                (Some(tstate.inbox_mmr()), Some(tstate.update_vk()))
            }
        };

        Self {
            asm_history_mmr,
            local_inbox_mmr,
            local_predicate_key,
        }
    }
}

/// Tracks context for verifying the various proofs from a tx.
pub(crate) struct TxProofsTracker<'a> {
    accumulator_proofs: Option<&'a RawMerkleProofList>,
    next_acc_proof_idx: usize,
    predicate_satisfiers: Option<&'a ProofSatisfierList>,
    next_pred_proof_idx: usize,
}

impl<'a> TxProofsTracker<'a> {
    pub(crate) fn from_txproofs(tx_proofs: &'a TxProofs) -> Self {
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

    fn inc_next_acc_proof(&mut self) {
        let cnt = self.acc_proofs_cnt();
        if self.next_acc_proof_idx == cnt {
            // FIXME return error
            panic!("ol/stf: overflow next acc proof idx");
        }

        self.next_acc_proof_idx += 1;
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

    fn inc_next_pred_proof(&mut self) {
        let cnt = self.pred_proofs_cnt();
        if self.next_pred_proof_idx == cnt {
            // FIXME return error
            panic!("ol/stf: overflow next pred proof idx");
        }

        self.next_pred_proof_idx += 1;
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
#[expect(missing_debug_implementations, reason = "contains opaque borrow types that don't implement Debug")]
pub struct TxProofVerifierImpl<'a> {
    state_ctx: TxProofVerificationContext<'a>,
    proof_tracker: TxProofsTracker<'a>,
}

impl<'a> TxProofVerifierImpl<'a> {
    /// Creates a new verifier from the account state context and proof tracker.
    pub(crate) fn new(
        state_ctx: TxProofVerificationContext<'a>,
        proof_tracker: TxProofsTracker<'a>,
    ) -> Self {
        Self {
            state_ctx,
            proof_tracker,
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

        if !mmr.verify(&indexed_proof, &leaf) {
            return Err(ProofVerifyError::InvalidProof);
        }

        self.proof_tracker.inc_next_acc_proof();
        Ok(())
    }
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

    fn verify_asm_history_mmr_proof_next(
        &mut self,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError> {
        self.verify_next_mmr_proof(self.state_ctx.asm_history_mmr, claim)
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
            .map_err(|_| ProofVerifyError::InvalidProof)?;

        self.proof_tracker.inc_next_pred_proof();
        Ok(())
    }

    fn is_exhausted(&self) -> bool {
        self.proof_tracker.is_all_done()
    }
}
