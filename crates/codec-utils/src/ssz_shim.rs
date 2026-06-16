//! Shim for encoding SSZ types with the [`Codec`] trait.

use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};

/// Maximum byte length for a single SSZ-encoded object (16 MiB).
const MAX_SSZ_OBJECT_BYTES: usize = 16 << 20;

/// Wraps an SSZ type so that it can be transparently [`Codec`]ed.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, Decode, Encode)]
#[ssz(struct_behaviour = "transparent")]
pub struct CodecSsz<T: Decode + Encode>(T);

impl<T: Decode + Encode> CodecSsz<T> {
    pub fn new(inner: T) -> Self {
        Self(inner)
    }

    pub fn inner(&self) -> &T {
        &self.0
    }

    pub fn into_inner(self) -> T {
        self.0
    }

    pub fn inner_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T: Decode + Encode> Codec for CodecSsz<T> {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Read a varint describing the length of the buffer.
        let len = Varint::decode(dec)?;
        let len_usize = len.inner() as usize;

        if len_usize > MAX_SSZ_OBJECT_BYTES {
            return Err(CodecError::OverflowContainer);
        }

        // Read a buffer of that size.
        let mut buffer = vec![0u8; len_usize];
        dec.read_buf(&mut buffer)?;

        // And then just decode it from the buffer.
        let inner = T::from_ssz_bytes(&buffer).map_err(|_| CodecError::MalformedField("ssz"))?;

        Ok(Self(inner))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        // Encode convert the inner value to SSZ.
        let bytes = self.0.as_ssz_bytes();

        if bytes.len() > MAX_SSZ_OBJECT_BYTES {
            return Err(CodecError::OverflowContainer);
        }

        // First encode the length as a varint.
        let len = Varint::new_usize(bytes.len()).ok_or(CodecError::OverflowContainer)?;
        len.encode(enc)?;
        enc.write_buf(&bytes)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Decode, Encode)]
    struct TestStruct {
        a: u32,
        b: u64,
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = TestStruct { a: 42, b: 1337 };
        let wrapped = CodecSsz::new(original.clone());

        // Encode to bytes
        let encoded = encode_to_vec(&wrapped).expect("Failed to encode");

        // Decode from bytes
        let decoded: CodecSsz<TestStruct> = decode_buf_exact(&encoded).expect("Failed to decode");

        // Check that we got the same value back
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_empty_encode_decode() {
        #[derive(Debug, Clone, PartialEq, Eq, Decode, Encode)]
        struct EmptyStruct;

        let original = EmptyStruct;
        let wrapped = CodecSsz::new(original.clone());

        // Encode to bytes
        let encoded = encode_to_vec(&wrapped).expect("Failed to encode");

        // Decode from bytes
        let decoded: CodecSsz<EmptyStruct> = decode_buf_exact(&encoded).expect("Failed to decode");

        // Check that we got the same value back
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_vector_encode_decode() {
        let original = vec![1u32, 2, 3, 4, 5];
        let wrapped = CodecSsz::new(original.clone());

        // Encode to bytes
        let encoded = encode_to_vec(&wrapped).expect("Failed to encode");

        // Decode from bytes
        let decoded: CodecSsz<Vec<u32>> = decode_buf_exact(&encoded).expect("Failed to decode");

        // Check that we got the same value back
        assert_eq!(decoded.inner(), &original);
    }
}
