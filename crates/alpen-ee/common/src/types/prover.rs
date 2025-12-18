use strata_acct_types::Hash;

/// Unique identifier to a persisted proof.
pub type ProofId = Hash;

/// TODO: proper proof type
#[expect(dead_code, reason = "wip")]
#[derive(Debug)]
pub struct Proof(Vec<u8>);
