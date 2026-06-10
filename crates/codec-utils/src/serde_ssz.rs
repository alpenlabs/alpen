//! Shim for exposing SSZ types through [`serde`].

use std::fmt;

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, Visitor},
};
use ssz::{Decode, Encode};

/// Wraps an SSZ type so that it can be transparently [`Serialize`]d and
/// [`Deserialize`]d.
///
/// The inner value is encoded to its SSZ byte representation. Binary formats
/// receive it as a raw byte blob (like a `Vec<u8>`), while human-readable
/// formats receive it as a hex-encoded string.
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub struct SerdeSsz<T>(T);

impl<T> SerdeSsz<T> {
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

impl<T: Encode> Serialize for SerdeSsz<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let bytes = self.0.as_ssz_bytes();

        if serializer.is_human_readable() {
            serializer.serialize_str(&hex::encode(&bytes))
        } else {
            serializer.serialize_bytes(&bytes)
        }
    }
}

impl<'de, T: Decode> Deserialize<'de> for SerdeSsz<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = if deserializer.is_human_readable() {
            let s = String::deserialize(deserializer)?;
            hex::decode(&s).map_err(de::Error::custom)?
        } else {
            deserializer.deserialize_bytes(BytesVisitor)?
        };

        let inner =
            T::from_ssz_bytes(&bytes).map_err(|e| de::Error::custom(format!("ssz ({e:?})")))?;

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
    use ssz_derive::{Decode, Encode};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Decode, Encode)]
    struct TestStruct {
        a: u32,
        b: u64,
    }

    #[test]
    fn test_json_roundtrip_is_hex_string() {
        let original = TestStruct { a: 42, b: 1337 };
        let wrapped = SerdeSsz::new(original.clone());

        let json = serde_json::to_string(&wrapped).expect("test: failed to serialize");

        // Human-readable formats encode as a hex string.
        let expected = format!("\"{}\"", hex::encode(original.as_ssz_bytes()));
        assert_eq!(json, expected);

        let decoded: SerdeSsz<TestStruct> =
            serde_json::from_str(&json).expect("test: failed to deserialize");
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_ciborium_roundtrip_is_binary() {
        let original = TestStruct { a: 42, b: 1337 };
        let wrapped = SerdeSsz::new(original.clone());

        let mut bytes = Vec::new();
        ciborium::into_writer(&wrapped, &mut bytes).expect("test: failed to serialize");

        let decoded: SerdeSsz<TestStruct> =
            ciborium::from_reader(bytes.as_slice()).expect("test: failed to deserialize");
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_vector_roundtrip() {
        let original = vec![1u32, 2, 3, 4, 5];
        let wrapped = SerdeSsz::new(original.clone());

        let json = serde_json::to_string(&wrapped).expect("test: failed to serialize");
        let decoded: SerdeSsz<Vec<u32>> =
            serde_json::from_str(&json).expect("test: failed to deserialize");
        assert_eq!(decoded.inner(), &original);

        let mut bytes = Vec::new();
        ciborium::into_writer(&wrapped, &mut bytes).expect("test: failed to serialize");
        let decoded: SerdeSsz<Vec<u32>> =
            ciborium::from_reader(bytes.as_slice()).expect("test: failed to deserialize");
        assert_eq!(decoded.inner(), &original);
    }

    #[test]
    fn test_invalid_hex_fails() {
        let bad = "\"not hex\"";
        let result: Result<SerdeSsz<TestStruct>, _> = serde_json::from_str(bad);
        assert!(result.is_err());
    }
}
