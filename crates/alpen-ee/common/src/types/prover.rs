use strata_acct_types::Hash;

/// Unique identifier to a proof.
pub type ProofId = Hash;

// TODO(STR-3682): proper proof type
#[derive(Debug)]
pub struct Proof(Vec<u8>);

impl Proof {
    pub fn from_vec(v: Vec<u8>) -> Self {
        Self(v)
    }

    pub fn to_vec(self) -> Vec<u8> {
        self.0
    }
}
