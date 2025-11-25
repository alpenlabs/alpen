use strata_acct_types::AccountSerial;
use strata_identifiers::{Buf32, hash::raw};

/// Log emitted during OL block execution.
#[derive(Clone, Debug)]
pub struct OLLog {
    /// Account this log is related to.
    // TODO should this actually be the ID and we can encode it in the diff more succinctly?
    account_serial: AccountSerial,

    /// Opaque log payload.
    payload: Vec<u8>,
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

    /// Computes the hash commitment of this log.
    /// TODO: This will use SSZ merkle hashing when ready.
    pub fn compute_hash_commitment(&self) -> Buf32 {
        // Serialize the log as account_serial (u32) + payload
        let mut data = Vec::new();
        data.extend_from_slice(&self.account_serial.inner().to_le_bytes());
        data.extend_from_slice(&self.payload);
        raw(&data)
    }
}
