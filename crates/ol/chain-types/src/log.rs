use ssz_types::VariableList;
use strata_acct_types::AccountSerial;
use strata_identifiers::Buf32;
use tree_hash::{Sha256Hasher, TreeHash};

use crate::ssz_generated::ssz::log::OLLog;

impl OLLog {
    pub fn new(account_serial: AccountSerial, payload: Vec<u8>) -> Self {
        Self {
            account_serial,
            payload: VariableList::new(payload).expect("log: payload too large"),
        }
    }

    pub fn account_serial(&self) -> AccountSerial {
        self.account_serial
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Computes the hash commitment of this log using SSZ tree hash.
    pub fn compute_hash_commitment(&self) -> Buf32 {
        let root = TreeHash::<Sha256Hasher>::tree_hash_root(self);
        Buf32::from(root.0)
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl<'a> arbitrary::Arbitrary<'a> for OLLog {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let account_serial = AccountSerial::from(u.arbitrary::<u32>()?);
        let payload_len = u.int_in_range(0..=1024)?;
        let payload: Vec<u8> = (0..payload_len)
            .map(|_| u.arbitrary())
            .collect::<arbitrary::Result<_>>()?;
        Ok(Self::new(account_serial, payload))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_acct_types::AccountSerial;
    use strata_test_utils_ssz::ssz_proptest;

    use super::OLLog;

    fn ollog_strategy() -> impl Strategy<Value = OLLog> {
        (
            any::<u32>().prop_map(AccountSerial::from),
            prop::collection::vec(any::<u8>(), 0..1024),
        )
            .prop_map(|(account_serial, payload)| OLLog::new(account_serial, payload))
    }

    mod ollog {
        use super::*;

        ssz_proptest!(OLLog, ollog_strategy());

        #[test]
        fn test_empty_payload() {
            let log = OLLog::new(AccountSerial::from(0), vec![]);
            let encoded = log.as_ssz_bytes();
            let decoded = OLLog::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(log.account_serial(), decoded.account_serial());
            assert_eq!(log.payload(), decoded.payload());
        }

        #[test]
        fn test_with_payload() {
            let log = OLLog::new(AccountSerial::from(42), vec![1, 2, 3, 4, 5]);
            let encoded = log.as_ssz_bytes();
            let decoded = OLLog::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(log.account_serial(), decoded.account_serial());
            assert_eq!(log.payload(), decoded.payload());
        }

        #[test]
        fn test_compute_hash_commitment() {
            let log1 = OLLog::new(AccountSerial::from(1), vec![1, 2, 3]);
            let log2 = OLLog::new(AccountSerial::from(1), vec![1, 2, 3]);
            let log3 = OLLog::new(AccountSerial::from(2), vec![1, 2, 3]);

            // Same log should produce same hash
            assert_eq!(
                log1.compute_hash_commitment(),
                log2.compute_hash_commitment()
            );

            // Different account serial should produce different hash
            assert_ne!(
                log1.compute_hash_commitment(),
                log3.compute_hash_commitment()
            );
        }
    }
}
