use strata_acct_types::AccountSerial;

/// Log emitted during OL block execution.
#[derive(Clone, Debug)]
pub struct OLLog {
    /// Account this log is related to.
    // TODO: Determine if this should be here or inside payload
    account_serial: AccountSerial,

    /// Opaque log payload.
    payload: Vec<u8>,
    // TODO: add more concrete fields.
}

impl OLLog {
    pub fn new(account_serial: AccountSerial, payload: Vec<u8>) -> Self {
        Self {
            account_serial,
            payload,
        }
    }

    pub fn account_serial(&self) -> AccountSerial {
        self.account_serial
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}
