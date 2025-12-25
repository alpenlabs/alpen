//! Variable-length subject identifier bytes for deposit requests.

use arbitrary::Arbitrary;
use strata_identifiers::{SUBJ_ID_LEN, SubjectId};
use thiserror::Error;

/// Error type for [`SubjectBytes`] operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubjectBytesError {
    /// Subject bytes exceed the maximum allowed length.
    #[error("subject bytes length {0} exceeds maximum length {1}")]
    TooLong(usize, usize),
}

/// Variable-length [`SubjectId`] bytes.
///
/// Subject IDs are canonically [`SUBJ_ID_LEN`] bytes per the account system specification, but in
/// practice many subject IDs are shorter. This type stores the variable-length byte representation
/// to optimize DA costs by avoiding unnecessary zero padding in the on-chain deposit descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectBytes(Vec<u8>);

impl SubjectBytes {
    /// Creates a new `SubjectBytes` instance from a byte vector.
    ///
    /// # Errors
    ///
    /// Returns an error if the length exceeds [`SUBJ_ID_LEN`].
    pub fn try_new(bytes: Vec<u8>) -> Result<Self, SubjectBytesError> {
        if bytes.len() > SUBJ_ID_LEN {
            return Err(SubjectBytesError::TooLong(bytes.len(), SUBJ_ID_LEN));
        }
        Ok(Self(bytes))
    }

    /// Returns the raw, unpadded subject bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Converts to a canonical [`SUBJ_ID_LEN`]-byte [`SubjectId`].
    ///
    /// This method allocates a [`SUBJ_ID_LEN`]-byte buffer and zero-pads the stored subject bytes.
    /// The original bytes are copied to the beginning of the buffer, with any remaining
    /// bytes filled with zeros.
    ///
    /// # Example
    ///
    /// If the stored bytes are shorter than [`SUBJ_ID_LEN`], such as `[0xAA, 0xBB, ..., 0xFF]`,
    /// this method returns a [`SUBJ_ID_LEN`]-byte `SubjectId` with the bytes at the start and
    /// trailing zeros: `[0xAA, 0xBB, ..., 0xFF, 0x00, 0x00, ..., 0x00]`.
    pub fn to_subject_id(&self) -> SubjectId {
        let mut buf = [0u8; SUBJ_ID_LEN];
        buf[..self.0.len()].copy_from_slice(&self.0);
        SubjectId::new(buf)
    }

    /// Returns the length of the subject bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns true if the subject bytes are empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the inner bytes, consuming the `SubjectBytes`.
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

impl<'a> Arbitrary<'a> for SubjectBytes {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate bytes with length between 0 and SUBJ_ID_LEN
        let len = u.int_in_range(0..=SUBJ_ID_LEN)?;
        let mut bytes = vec![0u8; len];
        u.fill_buffer(&mut bytes)?;
        // Safe to unwrap since we ensure len <= SUBJ_ID_LEN
        Ok(Self::try_new(bytes).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn prop_accepts_valid_length(bytes in prop::collection::vec(any::<u8>(), 0..=SUBJ_ID_LEN)) {
            let result = SubjectBytes::try_new(bytes.clone());
            prop_assert!(result.is_ok());
            let sb = result.unwrap();
            prop_assert_eq!(sb.as_bytes(), &bytes[..]);
            prop_assert_eq!(sb.len(), bytes.len());
            prop_assert_eq!(sb.is_empty(), bytes.is_empty());
        }

        #[test]
        fn prop_rejects_too_long(
            bytes in prop::collection::vec(any::<u8>(), (SUBJ_ID_LEN + 1)..=(SUBJ_ID_LEN + 100))
        ) {
            let len = bytes.len();
            let result = SubjectBytes::try_new(bytes);
            prop_assert!(result.is_err());
            prop_assert!(matches!(result, Err(SubjectBytesError::TooLong(actual, expected))
                if actual == len && expected == SUBJ_ID_LEN));
        }

        #[test]
        fn prop_to_subject_id_preserves_and_pads(bytes in prop::collection::vec(any::<u8>(), 0..=SUBJ_ID_LEN)) {
            let sb = SubjectBytes::try_new(bytes.clone()).unwrap();
            let subject_id = sb.to_subject_id();
            let inner = subject_id.inner();

            // Original bytes should be preserved at the start
            prop_assert_eq!(&inner[..bytes.len()], &bytes[..]);

            // Remaining bytes should be zeros (padding)
            for &byte in &inner[bytes.len()..] {
                prop_assert_eq!(byte, 0);
            }

            // Total length should always be SUBJ_ID_LEN
            prop_assert_eq!(inner.len(), SUBJ_ID_LEN);
        }
    }
}
