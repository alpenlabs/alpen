//! Types for manipulating proof steps.

use strata_acct_types::{AccumulatorClaim, Mmr64};
use strata_predicate::PredicateKey;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProofVerifyError {
    #[error("no next proof")]
    NoNextProof,

    #[error("invalid proof")]
    InvalidProof,
}

/// Describes an opaque verifier that takes claim checks we want to make.
///
/// This is expected to walk over the proof fields in a transaction.  This
/// should also help us when signing txs.
pub trait TxProofVerifier {
    /// Verifies the next MMR proof with the root and claim.
    fn verify_next_mmr_proof(
        &mut self,
        root: &Mmr64,
        claim: &AccumulatorClaim,
    ) -> Result<(), ProofVerifyError>;

    /// Verifies the next predicate proof against a provided key and claim.
    fn verify_next_predicate_satisfier(
        &mut self,
        key: &PredicateKey,
        claim: &[u8],
    ) -> Result<(), ProofVerifyError>;

    /// Returns true if all proofs have been fully exhausted.
    fn is_exhausted(&self) -> bool;
}
