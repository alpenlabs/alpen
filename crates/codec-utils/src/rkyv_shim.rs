//! Shim for encoding rkyv types with the [`Codec`] trait.

use rkyv::{
    Archive, Archived, Deserialize, Serialize,
    api::high::{HighDeserializer, HighSerializer},
    rancor::Error as RkyvError,
    ser::allocator::ArenaHandle,
    util::AlignedVec,
};
use strata_codec::{Codec, CodecError, Decoder, Encoder, Varint};

pub fn decode_rkyv<T>(bytes: &[u8]) -> Result<T, RkyvError>
where
    T: Archive,
    Archived<T>: Deserialize<T, HighDeserializer<RkyvError>>,
{
    rkyv::from_bytes::<T, RkyvError>(bytes)
}

/// Wraps an rkyv type so that it can be transparently [`Codec`]ed.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub struct CodecRkyv<T>(pub T);

impl<T> CodecRkyv<T> {
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

impl<T> Codec for CodecRkyv<T>
where
    T: Archive + for<'a> Serialize<HighSerializer<AlignedVec, ArenaHandle<'a>, RkyvError>>,
    Archived<T>: Deserialize<T, HighDeserializer<RkyvError>>,
{
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        // Read a varint describing the length of the buffer.
        let len = Varint::decode(dec)?;
        let len_usize = len.inner() as usize;

        // Read a buffer of that size.
        let mut buffer = vec![0u8; len_usize];
        dec.read_buf(&mut buffer)?;

        let inner = decode_rkyv::<T>(&buffer).map_err(|_| CodecError::MalformedField("rkyv"))?;

        Ok(Self(inner))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let bytes =
            rkyv::to_bytes::<RkyvError>(&self.0).map_err(|_| CodecError::MalformedField("rkyv"))?;

        let len = Varint::new_usize(bytes.len()).ok_or(CodecError::OverflowContainer)?;
        len.encode(enc)?;
        enc.write_buf(bytes.as_ref())?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize, Deserialize)]
    struct TestStruct {
        a: u32,
        b: u64,
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = TestStruct { a: 42, b: 1337 };
        let wrapped = CodecRkyv::new(original.clone());

        let encoded = encode_to_vec(&wrapped).expect("Failed to encode");
        let decoded: CodecRkyv<TestStruct> = decode_buf_exact(&encoded).expect("Failed to decode");

        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_empty_encode_decode() {
        #[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize, Deserialize)]
        struct EmptyStruct;

        let original = EmptyStruct;
        let wrapped = CodecRkyv::new(original.clone());

        let encoded = encode_to_vec(&wrapped).expect("Failed to encode");
        let decoded: CodecRkyv<EmptyStruct> = decode_buf_exact(&encoded).expect("Failed to decode");

        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_vector_encode_decode() {
        let original = vec![1u32, 2, 3, 4, 5];
        let wrapped = CodecRkyv::new(original.clone());

        let encoded = encode_to_vec(&wrapped).expect("Failed to encode");
        let decoded: CodecRkyv<Vec<u32>> = decode_buf_exact(&encoded).expect("Failed to decode");

        assert_eq!(decoded.inner(), &original);
    }
}
