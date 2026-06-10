use ssz_types::VariableList;
use strata_acct_types::AccountSerial;
use strata_asm_proto_checkpoint_types::{OLLogDecodeError, OLLogType};
use strata_codec::{decode_buf_exact, encode_to_vec};
use strata_identifiers::Buf32;
use strata_msg_fmt::{Msg, MsgRef, OwnedMsg, TypeId};
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

    /// Builds an [`OLLog`] whose payload is the msg-fmt envelope for a typed OL log.
    ///
    /// The payload is `TypeId(T::TY) ++ ssz(log)`, so consumers dispatch on the log type via
    /// [`OLLog::try_into_log`]. Mirrors `OLLog::from_log` in
    /// [`strata_asm_proto_checkpoint_types`].
    pub fn from_log<T: OLLogType>(
        account_serial: AccountSerial,
        log: &T,
    ) -> Result<Self, OLLogDecodeError> {
        let body = encode_to_vec(log)?;
        let payload = OwnedMsg::new(T::TY, body)?.to_vec();
        Ok(Self::new(account_serial, payload))
    }

    /// Tries to interpret the payload bytes as a msg-fmt message.
    ///
    /// Returns `None` if the payload is not a valid envelope.
    pub fn try_as_msg(&self) -> Option<MsgRef<'_>> {
        MsgRef::try_from(self.payload()).ok()
    }

    /// Returns the envelope type id, if the payload is a valid msg-fmt message.
    pub fn ty(&self) -> Option<TypeId> {
        self.try_as_msg().map(|msg| msg.ty())
    }

    /// Decodes the payload as a specific typed OL log.
    ///
    /// Parses the msg-fmt envelope, checks the type id matches `T::TY`, and decodes the body.
    /// Returns [`OLLogDecodeError::TypeMismatch`] when the envelope carries a different log type.
    pub fn try_into_log<T: OLLogType>(&self) -> Result<T, OLLogDecodeError> {
        let msg = MsgRef::try_from(self.payload())?;
        let found = msg.ty();
        if found != T::TY {
            return Err(OLLogDecodeError::TypeMismatch {
                expected: T::TY,
                found,
            });
        }
        Ok(decode_buf_exact(msg.body())?)
    }

    /// Computes the hash commitment of this log using SSZ tree hash.
    pub fn compute_hash_commitment(&self) -> Buf32 {
        let root = TreeHash::tree_hash_root::<Sha256Hasher>(self);
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
    use strata_asm_proto_checkpoint_types::{
        OLLogDecodeError, SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID, SimpleWithdrawalIntentLogData,
    };
    use strata_test_utils_ssz::ssz_proptest;

    use super::OLLog;
    use crate::{SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID, SnarkAccountUpdateLogData};

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

    /// Exercises the typed-log envelope round-trip carried by [`OLLog`], using a checkpoint-shared
    /// payload type ([`SimpleWithdrawalIntentLogData`]) and an OL-block-local one
    /// ([`SnarkAccountUpdateLogData`]).
    mod typed_log {
        use super::*;

        #[test]
        fn test_withdrawal_envelope_round_trip() {
            let serial = AccountSerial::from(7u32);
            let log_data = SimpleWithdrawalIntentLogData::new(100_000, b"bc1qdest".to_vec(), 3)
                .expect("dest within bounds");

            let log = OLLog::from_log(serial, &log_data).expect("from_log should succeed");
            assert_eq!(log.account_serial(), serial);
            assert_eq!(log.ty(), Some(SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID));

            let decoded = log
                .try_into_log::<SimpleWithdrawalIntentLogData>()
                .expect("try_into_log should succeed");
            assert_eq!(decoded, log_data);
        }

        #[test]
        fn test_snark_envelope_round_trip() {
            let serial = AccountSerial::from(9u32);
            let log_data =
                SnarkAccountUpdateLogData::new(42, b"extra".to_vec()).expect("within bounds");

            let log = OLLog::from_log(serial, &log_data).expect("from_log should succeed");
            assert_eq!(log.ty(), Some(SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID));

            let decoded = log
                .try_into_log::<SnarkAccountUpdateLogData>()
                .expect("try_into_log should succeed");
            assert_eq!(decoded, log_data);
        }

        #[test]
        fn test_envelope_type_mismatch() {
            let withdrawal = SimpleWithdrawalIntentLogData::new(10, b"d".to_vec(), 1)
                .expect("dest within bounds");
            let log = OLLog::from_log(AccountSerial::from(0u32), &withdrawal)
                .expect("from_log should succeed");

            // Decoding as the wrong log type reports a type mismatch rather than a spurious decode.
            let err = log
                .try_into_log::<SnarkAccountUpdateLogData>()
                .expect_err("decoding as the wrong type should fail");
            assert!(matches!(
                err,
                OLLogDecodeError::TypeMismatch { expected, found }
                    if expected == SNARK_ACCOUNT_UPDATE_LOG_TYPE_ID
                        && found == SIMPLE_WITHDRAWAL_INTENT_LOG_TYPE_ID
            ));

            // And decoding as the right type still works.
            log.try_into_log::<SimpleWithdrawalIntentLogData>()
                .expect("try_into_log should succeed");
        }

        #[test]
        fn test_non_envelope_payload_is_not_a_msg() {
            // A raw, non-envelope payload has no type id and fails typed decoding.
            let log = OLLog::new(AccountSerial::from(1u32), vec![]);
            assert_eq!(log.ty(), None);
            assert!(matches!(
                log.try_into_log::<SimpleWithdrawalIntentLogData>(),
                Err(OLLogDecodeError::Envelope(_))
            ));
        }
    }
}
