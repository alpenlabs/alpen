use strata_acct_types::AccountId;
use strata_primitives::Buf32;

/// Log emitted during OL block execution.
#[derive(Clone, Debug)]
pub struct OLLog {
    /// Account this log is related to.
    // TODO: maybe use account serial,
    account_id: AccountId,

    /// Opaque log payload.
    payload: Vec<u8>,
    // TODO: add more concrete fields.
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
}

pub fn compute_logs_root(_logs: &[OLLog]) -> Buf32 {
    // TODO: this will be ssz
    todo!()
}
