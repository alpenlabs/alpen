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
