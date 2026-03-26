//! Types for manipulating proof steps.

use strata_acct_types::AccumulatorClaim;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProofVerifyError {
    #[error("no next proof")]
    NoNextProof,

    #[error("invalid proof")]
    InvalidProof,

    /// This would be use if we tried to check an account input mmr in a context
    /// where that doesn't exit.
    #[error("proof impossible in this context")]
    InvalidContext,
}

/// Describes an opaque verifier that takes claim checks we want to make.
///
/// This is expected to walk over the proof fields in a transaction.
///
/// This will also help us when signing txs, as we can figure out the proofs
/// that we need to generate by looking and recording the calls that get made
/// and returning `Ok(())`.  This avoids more complex introspection into the tx.
pub trait TxProofVerifier {
    /// Verifies an account-local inbox MMR proof.
    fn verify_inbox_mmr_proof_next(
        &mut self,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError>;

    /// Verifies an ASM history MMR proof.
    fn verify_asm_history_mmr_proof_next(
        &mut self,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError>;

    /// Verifies the next predicate proof against the account-local predicate and claim.
    fn verify_local_predicate_next(&mut self, claim: &[u8]) -> Result<(), ProofVerifyError>;

    /// Returns true if all proofs available to verify have been fully exhausted.
    fn is_exhausted(&self) -> bool;
}

/// Impl of [`TxProofVerifier`] that doesn't actually verify proofs but indexes
/// what's being checked so we don't have to introspect the txs.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TxProofIndexer {
    accumulator_checks: Vec<AccProofCheck>,
    predicate_checks: Vec<PredicateCheck>,
}

impl TxProofIndexer {
    /// Creates a new fresh indexer.
    pub fn new_fresh() -> Self {
        Self::default()
    }

    pub fn accumulator_checks(&self) -> &[AccProofCheck] {
        &self.accumulator_checks
    }

    pub fn predicate_checks(&self) -> &[PredicateCheck] {
        &self.predicate_checks
    }
}

impl TxProofVerifier for TxProofIndexer {
    fn verify_inbox_mmr_proof_next(
        &mut self,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError> {
        self.accumulator_checks
            .push(AccProofCheck::Inbox(claim.clone()));
        Ok(())
    }

    fn verify_asm_history_mmr_proof_next(
        &mut self,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError> {
        self.accumulator_checks
            .push(AccProofCheck::AsmHistory(claim.clone()));
        Ok(())
    }

    fn verify_local_predicate_next(&mut self, claim: &[u8]) -> Result<(), ProofVerifyError> {
        self.predicate_checks
            .push(PredicateCheck::new(PredicateRef::Local, claim.to_vec()));
        Ok(())
    }

    fn is_exhausted(&self) -> bool {
        // hmm maybe we should be smarter about this?
        true
    }
}

/// Describes an accumulator proof check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccProofCheck {
    Inbox(AccumulatorClaim),
    AsmHistory(AccumulatorClaim),
}

/// Describes a ref to some predicate that exists in the context of the tx
/// verification.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PredicateRef {
    Local,
}

/// Describes a check on some predicate (by ref) with a particular claim.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PredicateCheck {
    pred_ref: PredicateRef,
    claim: Vec<u8>,
}

impl PredicateCheck {
    pub fn new(pred_ref: PredicateRef, claim: Vec<u8>) -> Self {
        Self { pred_ref, claim }
    }

    pub fn pred_ref(&self) -> PredicateRef {
        self.pred_ref
    }

    pub fn claim(&self) -> &[u8] {
        &self.claim
    }
}
