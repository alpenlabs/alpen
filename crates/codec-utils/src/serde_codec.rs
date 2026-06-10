//! Shim for exposing [`Codec`] types through [`serde`].

use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Visitor},
    ser,
};
use strata_codec::{Codec, decode_buf_exact, encode_to_vec};

/// Wraps a [`Codec`] type so that it can be transparently [`Serialize`]d and
/// [`Deserialize`]d.
///
/// The inner value is encoded to its [`Codec`] byte representation. Binary
/// formats receive it as a raw byte blob (like a `Vec<u8>`), while
/// human-readable formats receive it as a hex-encoded string.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub struct SerdeCodec<T>(T);

impl<T> SerdeCodec<T> {
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

impl<T: Codec> Serialize for SerdeCodec<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let bytes = encode_to_vec(&self.0).map_err(ser::Error::custom)?;

        if serializer.is_human_readable() {
            serializer.serialize_str(&hex::encode(&bytes))
        } else {
            serializer.serialize_bytes(&bytes)
        }
    }
}

impl<'de, T: Codec> Deserialize<'de> for SerdeCodec<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            hex::decode(&s).map_err(de::Error::custom)?
        } else {
            deserializer.deserialize_bytes(BytesVisitor)?
        };

        let inner = decode_buf_exact(&bytes).map_err(de::Error::custom)?;

        Ok(Self(inner))
    }
}

/// Collects an arbitrary byte blob, accepting whatever shape the underlying
/// binary format uses to represent bytes.
struct BytesVisitor;

impl<'de> Visitor<'de> for BytesVisitor {
    type Value = Vec<u8>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a byte blob")
    }

    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Self::Value, E> {
        Ok(v.to_vec())
    }

    fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Self::Value, E> {
        Ok(v)
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut buf = Vec::with_capacity(seq.size_hint().unwrap_or(0));
        while let Some(byte) = seq.next_element()? {
            buf.push(byte);
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{Codec, CodecError, Decoder, Encoder, VarVec};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestStruct {
        a: u32,
        b: u64,
    }

    impl Codec for TestStruct {
        fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
            let a = u32::decode(dec)?;
            let b = u64::decode(dec)?;
            Ok(Self { a, b })
        }

        fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
            self.a.encode(enc)?;
            self.b.encode(enc)?;
            Ok(())
        }
    }

    #[test]
    fn test_json_roundtrip_is_hex_string() {
        let original = TestStruct { a: 42, b: 1337 };
        let wrapped = SerdeCodec::new(original.clone());

        let json = serde_json::to_string(&wrapped).expect("failed to serialize");

        // Human-readable formats encode as a hex string.
        let expected = format!("\"{}\"", hex::encode(encode_to_vec(&original).unwrap()));
        assert_eq!(json, expected);

        let decoded: SerdeCodec<TestStruct> =
            serde_json::from_str(&json).expect("failed to deserialize");
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_ciborium_roundtrip_is_binary() {
        let original = TestStruct { a: 42, b: 1337 };
        let wrapped = SerdeCodec::new(original.clone());

        let mut bytes = Vec::new();
        ciborium::into_writer(&wrapped, &mut bytes).expect("failed to serialize");

        let decoded: SerdeCodec<TestStruct> =
            ciborium::from_reader(bytes.as_slice()).expect("failed to deserialize");
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_vector_roundtrip() {
        let original: VarVec<u32> = VarVec::from_vec(vec![1u32, 2, 3, 4, 5]).unwrap();
        let wrapped = SerdeCodec::new(original.clone());

        let json = serde_json::to_string(&wrapped).expect("failed to serialize");
        let decoded: SerdeCodec<VarVec<u32>> =
            serde_json::from_str(&json).expect("failed to deserialize");
        assert_eq!(decoded.inner(), &original);

        let mut bytes = Vec::new();
        ciborium::into_writer(&wrapped, &mut bytes).expect("failed to serialize");
        let decoded: SerdeCodec<VarVec<u32>> =
            ciborium::from_reader(bytes.as_slice()).expect("failed to deserialize");
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_invalid_hex_fails() {
        let bad = "\"not hex\"";
        let result: Result<SerdeCodec<TestStruct>, _> = serde_json::from_str(bad);
        assert!(result.is_err());
    }
}
