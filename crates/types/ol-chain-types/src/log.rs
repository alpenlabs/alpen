use sha2::{Digest, Sha256};
use strata_acct_types::AccountId;
use strata_primitives::buf::Buf32;

/// A log entry for an account operation
#[derive(Debug, Clone)]
pub struct OLLog {
    account_id: AccountId,
    payload: Vec<u8>, // TODO: make this typed, serialization can be done at the edges
}

impl OLLog {
    pub fn new(account_id: AccountId, payload: Vec<u8>) -> Self {
        Self {
            account_id,
            payload,
        }
    }

    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    // NOTE: This will also be redundant after SSZ
    pub fn compute_root(logs: &[Self]) -> Buf32 {
        let mut hasher = Sha256::new();
        for log in logs {
            hasher.update(log.account_id().inner());
            hasher.update(log.payload());
        }
        Buf32::new(hasher.finalize().into())
    }
}
