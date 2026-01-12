//! Codec wrappers for Alloy types.

use alloy_primitives::U256;
use revm_primitives::{Address, B256};
use serde::{Deserialize, Serialize};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Wrapper for U256 that implements `Codec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecU256(pub U256);

impl Codec for CodecU256 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(&self.0.to_le_bytes::<32>())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mut buf = [0u8; 32];
        dec.read_buf(&mut buf)?;
        Ok(Self(U256::from_le_bytes(buf)))
    }
}

impl From<U256> for CodecU256 {
    fn from(v: U256) -> Self {
        Self(v)
    }
}

impl From<CodecU256> for U256 {
    fn from(v: CodecU256) -> Self {
        v.0
    }
}

/// Wrapper for B256 that implements `Codec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodecB256(pub B256);

impl Codec for CodecB256 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(self.0.as_slice())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mut buf = [0u8; 32];
        dec.read_buf(&mut buf)?;
        Ok(Self(B256::from(buf)))
    }
}

impl From<B256> for CodecB256 {
    fn from(v: B256) -> Self {
        Self(v)
    }
}

impl From<CodecB256> for B256 {
    fn from(v: CodecB256) -> Self {
        v.0
    }
}

/// Wrapper for Address that implements `Codec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CodecAddress(pub Address);

impl Codec for CodecAddress {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(self.0.as_slice())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mut buf = [0u8; 20];
        dec.read_buf(&mut buf)?;
        Ok(Self(Address::from(buf)))
    }
}

impl From<Address> for CodecAddress {
    fn from(v: Address) -> Self {
        Self(v)
    }
}

impl From<CodecAddress> for Address {
    fn from(v: CodecAddress) -> Self {
        v.0
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    #[test]
    fn test_codec_u256_roundtrip() {
        let val = CodecU256(U256::from(0x1234567890abcdefu64));
        let encoded = encode_to_vec(&val).unwrap();
        let decoded: CodecU256 = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }

    #[test]
    fn test_codec_b256_roundtrip() {
        let val = CodecB256(B256::from([0x42u8; 32]));
        let encoded = encode_to_vec(&val).unwrap();
        let decoded: CodecB256 = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }
}
