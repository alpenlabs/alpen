#[cfg(feature = "arbitrary")]
use arbitrary::Arbitrary;
// Re-export from the separate logs crate
use strata_codec::{Codec, decode_buf_exact};
use strata_msg_fmt::{Msg, MsgRef, OwnedMsg, TypeId};

use crate::{AsmManifestError, AsmManifestResult, ssz_generated::ssz::log::AsmLogEntry};

/// Trait for ASM log types that can be serialized and stored.
///
/// This trait provides a consistent interface for log entries that need to be
/// serialized, stored, and later deserialized from the ASM state. Each log type
/// has a unique type identifier and must be serializable.
pub trait AsmLog: Codec {
    /// Unique type identifier for this log type.
    ///
    /// This constant is used to distinguish between different log types when
    /// serializing and deserializing log entries.
    const TY: TypeId;
}

impl AsmLogEntry {
    /// Create an AsmLogEntry directly from raw bytes.
    ///
    /// This is the most basic constructor - logs are just bytes.
    pub fn from_raw(bytes: Vec<u8>) -> Self {
        AsmLogEntry { data: bytes.into() }
    }

    /// Create an AsmLogEntry from SPS-52 message components.
    ///
    /// This creates a properly formatted SPS-52 message with type ID and body.
    pub fn from_msg(ty: TypeId, body: Vec<u8>) -> AsmManifestResult<Self> {
        let owned_msg = OwnedMsg::new(ty, body)?;
        Ok(AsmLogEntry {
            data: owned_msg.to_vec().into(),
        })
    }

    /// Create an AsmLogEntry from any type that implements AsmLog.
    ///
    /// This provides backwards compatibility with typed log entries.
    pub fn from_log<T: AsmLog>(log: &T) -> AsmManifestResult<Self> {
        let ty = TypeId::from(T::TY);
        use strata_codec::encode_to_vec;
        let body = encode_to_vec(log)?;
        Self::from_msg(ty, body)
    }

    /// Try to interpret the raw bytes as an SPS-52 message.
    ///
    /// Returns None if the bytes don't form a valid SPS-52 message.
    /// This allows logs to be either structured messages or arbitrary bytes.
    pub fn try_as_msg(&self) -> Option<MsgRef<'_>> {
        MsgRef::try_from(&self.data[..]).ok()
    }

    /// Get the type ID if this is a valid SPS-52 message.
    ///
    /// Returns None if the log is not a valid message.
    pub fn ty(&self) -> Option<TypeId> {
        self.try_as_msg().map(|msg| msg.ty())
    }

    /// Try to deserialize the log entry to a specific AsmLog type.
    ///
    /// This only works if the log is a valid SPS-52 message with the correct type ID.
    pub fn try_into_log<T: AsmLog>(&self) -> AsmManifestResult<T> {
        // Parse as message, propagating any parsing errors
        let msg = MsgRef::try_from(&self.data[..])?;

        let expected_ty = T::TY;
        let actual_ty = msg.ty();

        if actual_ty != expected_ty {
            return Err(AsmManifestError::TypeIdMismatch(crate::Mismatched {
                expected: expected_ty,
                actual: actual_ty,
            }));
        }

        decode_buf_exact(msg.body()).map_err(AsmManifestError::from)
    }

    /// Get the raw bytes of this log entry.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume the log entry and return the raw bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.data.into()
    }
}

// Manual Arbitrary implementation for testing/benchmarking
#[cfg(feature = "arbitrary")]
impl<'a> Arbitrary<'a> for AsmLogEntry {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate a small random byte vec for the log data
        let len = u.int_in_range(0..=256)?;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            bytes.push(u8::arbitrary(u)?);
        }
        Ok(AsmLogEntry::from_raw(bytes))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use ssz::{Decode, Encode};
    use strata_test_utils_ssz::ssz_proptest;

    use super::AsmLogEntry;

    fn asm_log_entry_strategy() -> impl Strategy<Value = AsmLogEntry> {
        prop::collection::vec(any::<u8>(), 0..1024).prop_map(AsmLogEntry::from_raw)
    }

    mod asm_log_entry {
        use super::*;

        ssz_proptest!(AsmLogEntry, asm_log_entry_strategy());

        #[test]
        fn test_empty_data() {
            let log = AsmLogEntry::from_raw(vec![]);
            let encoded = log.as_ssz_bytes();
            let decoded = AsmLogEntry::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(log.as_bytes(), decoded.as_bytes());
        }

        #[test]
        fn test_with_data() {
            let log = AsmLogEntry::from_raw(vec![1, 2, 3, 4, 5]);
            let encoded = log.as_ssz_bytes();
            let decoded = AsmLogEntry::from_ssz_bytes(&encoded).unwrap();
            assert_eq!(log.as_bytes(), decoded.as_bytes());
        }

        #[test]
        fn test_from_raw_roundtrip() {
            let data = vec![42u8; 100];
            let log = AsmLogEntry::from_raw(data.clone());
            assert_eq!(log.as_bytes(), &data);
            assert_eq!(log.into_bytes(), data);
        }
    }
}
