//! Codec re-exports and helpers from strata-codec.

use std::collections::BTreeMap;

// Re-export everything from strata-codec
pub use strata_codec::{
    Codec, CodecError, Decoder, Encoder, Varint, decode_buf_exact, encode_to_vec,
};

// Create type alias for Result
pub type CodecResult<T> = Result<T, CodecError>;

// Std collections encoding/decoding helpers

/// Encodes a BTreeMap where both key and value implement [`Codec`].
pub fn encode_map<K: Codec, V: Codec>(
    map: &BTreeMap<K, V>,
    enc: &mut impl Encoder,
) -> Result<(), CodecError> {
    (map.len() as u32).encode(enc)?;
    for (k, v) in map {
        k.encode(enc)?;
        v.encode(enc)?;
    }
    Ok(())
}

/// Decodes a BTreeMap where both key and value implement [`Codec`].
pub fn decode_map<K: Codec + Ord, V: Codec>(
    dec: &mut impl Decoder,
) -> Result<BTreeMap<K, V>, CodecError> {
    let count = u32::decode(dec)? as usize;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let k = K::decode(dec)?;
        let v = V::decode(dec)?;
        map.insert(k, v);
    }
    Ok(map)
}

/// Encodes a Vec where elements implement [`Codec`].
pub fn encode_vec<T: Codec>(vec: &[T], enc: &mut impl Encoder) -> Result<(), CodecError> {
    (vec.len() as u32).encode(enc)?;
    for item in vec {
        item.encode(enc)?;
    }
    Ok(())
}

/// Decodes a Vec where elements implement [`Codec`].
pub fn decode_vec<T: Codec>(dec: &mut impl Decoder) -> Result<Vec<T>, CodecError> {
    let count = u32::decode(dec)? as usize;
    let mut vec = Vec::with_capacity(count);
    for _ in 0..count {
        vec.push(T::decode(dec)?);
    }
    Ok(vec)
}

/// Encodes a BTreeMap with key/value conversion functions.
///
/// Use when keys or values need wrapping before encoding.
pub fn encode_map_with<K, V, CK, CV>(
    map: &BTreeMap<K, V>,
    enc: &mut impl Encoder,
    encode_key: impl Fn(&K) -> CK,
    encode_value: impl Fn(&V) -> CV,
) -> Result<(), CodecError>
where
    CK: Codec,
    CV: Codec,
{
    (map.len() as u32).encode(enc)?;
    for (k, v) in map {
        encode_key(k).encode(enc)?;
        encode_value(v).encode(enc)?;
    }
    Ok(())
}

/// Decodes a BTreeMap with key/value conversion functions.
///
/// Use when keys or values need unwrapping after decoding.
pub fn decode_map_with<K, V, CK, CV>(
    dec: &mut impl Decoder,
    decode_key: impl Fn(CK) -> K,
    decode_value: impl Fn(CV) -> V,
) -> Result<BTreeMap<K, V>, CodecError>
where
    K: Ord,
    CK: Codec,
    CV: Codec,
{
    let count = u32::decode(dec)? as usize;
    let mut map = BTreeMap::new();
    for _ in 0..count {
        let k = decode_key(CK::decode(dec)?);
        let v = decode_value(CV::decode(dec)?);
        map.insert(k, v);
    }
    Ok(map)
}

/// Encodes a Vec with element conversion.
pub fn encode_vec_with<T, CT>(
    vec: &[T],
    enc: &mut impl Encoder,
    encode_elem: impl Fn(&T) -> CT,
) -> Result<(), CodecError>
where
    CT: Codec,
{
    (vec.len() as u32).encode(enc)?;
    for item in vec {
        encode_elem(item).encode(enc)?;
    }
    Ok(())
}

/// Decodes a Vec with element conversion.
pub fn decode_vec_with<T, CT>(
    dec: &mut impl Decoder,
    decode_elem: impl Fn(CT) -> T,
) -> Result<Vec<T>, CodecError>
where
    CT: Codec,
{
    let count = u32::decode(dec)? as usize;
    let mut vec = Vec::with_capacity(count);
    for _ in 0..count {
        vec.push(decode_elem(CT::decode(dec)?));
    }
    Ok(vec)
}
