//! Shim for encoding Borsh types with the [`Codec`] trait.

use borsh::{BorshDeserialize, BorshSerialize};
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};

/// Wraps a Borsh type so that it can be transparently [`Codec`]ed.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, BorshSerialize, BorshDeserialize)]
pub struct CodecBorsh<T: BorshSerialize + BorshDeserialize>(pub T);

impl<T: BorshSerialize + BorshDeserialize> CodecBorsh<T> {
    pub fn new(inner: T) -> Self {
        Self(inner)
    }

    pub fn inner(&self) -> &T {
        &self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: BorshSerialize + BorshDeserialize> Codec for CodecBorsh<T> {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Read a varint describing the length of the buffer.
        let len = Varint::decode(dec)?;
        let len_usize = len.inner() as usize;

        // Read a buffer of that size.
        let mut buffer = vec![0u8; len_usize];
        dec.read_buf(&mut buffer)?;

        // And then just decode it from the buffer.
        let inner = T::try_from_slice(&buffer).map_err(|_| CodecError::MalformedField("borsh"))?;

        Ok(Self(inner))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode convert the inner value to Borsh.
        let bytes = borsh::to_vec(&self.0).map_err(|_| CodecError::MalformedField("borsh"))?;

        // First encode the length as a varint.
        let len = Varint::new_usize(bytes.len()).ok_or(CodecError::OverflowContainer)?;
        len.encode(enc)?;
        enc.write_buf(&bytes)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use borsh::{BorshDeserialize, BorshSerialize};
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
    struct TestStruct {
        a: u32,
        b: u64,
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = TestStruct { a: 42, b: 1337 };
        let wrapped = CodecBorsh::new(original.clone());

        // Encode to bytes
        let encoded = encode_to_vec(&wrapped).expect("Failed to encode");

        // Decode from bytes
        let decoded: CodecBorsh<TestStruct> = decode_buf_exact(&encoded).expect("Failed to decode");

        // Check that we got the same value back
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_empty_encode_decode() {
        #[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
        struct EmptyStruct;

        let original = EmptyStruct;
        let wrapped = CodecBorsh::new(original.clone());

        // Encode to bytes
        let encoded = encode_to_vec(&wrapped).expect("Failed to encode");

        // Decode from bytes
        let decoded: CodecBorsh<EmptyStruct> =
            decode_buf_exact(&encoded).expect("Failed to decode");

        // Check that we got the same value back
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_vector_encode_decode() {
        let original = vec![1u32, 2, 3, 4, 5];
        let wrapped = CodecBorsh::new(original.clone());

        // Encode to bytes
        let encoded = encode_to_vec(&wrapped).expect("Failed to encode");

        // Decode from bytes
        let decoded: CodecBorsh<Vec<u32>> = decode_buf_exact(&encoded).expect("Failed to decode");

        // Check that we got the same value back
        assert_eq!(decoded.inner(), &original);
    }
}
