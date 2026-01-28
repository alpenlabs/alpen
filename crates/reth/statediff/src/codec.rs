//! Codec wrappers for Alloy types.

use alloy_primitives::U256;
use revm_primitives::{Address, B256};
use strata_codec::{Codec, CodecError, Decoder, Encoder};

/// Trimmed U256 encoding - strips leading zeros for space efficiency.
///
/// Encoding format:
/// - `len: u8` (0-32) - number of significant bytes
/// - `data: [u8; len]` - big-endian bytes (no leading zeros)
///
/// Special case: `len=0` means value is zero.
///
/// **Note:** This type is available but not currently used for storage keys by default.
/// Storage keys are typically keccak256 hashes (uniformly distributed), so trimming
/// would add 1-byte overhead in most cases. See [`TrimmedStorageValue`] which is
/// used for storage values where trimming provides significant savings.
/// However, it potentially can be used for storages that use simple variables and small arrays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrimmedU256(pub U256);

impl Codec for TrimmedU256 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let bytes = self.0.to_be_bytes::<32>();
        // Find first non-zero byte
        let start = bytes.iter().position(|&b| b != 0).unwrap_or(32);
        let len = (32 - start) as u8;
        enc.write_buf(&[len])?;
        if len > 0 {
            enc.write_buf(&bytes[start..])?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let [len] = dec.read_arr::<1>()?;
        let len = len as usize;

        if len > 32 {
            return Err(CodecError::MalformedField("TrimmedU256 length exceeds 32"));
        }

        if len == 0 {
            return Ok(Self(U256::ZERO));
        }

        let mut buf = [0u8; 32];
        dec.read_buf(&mut buf[32 - len..])?;
        Ok(Self(U256::from_be_bytes(buf)))
    }
}

impl From<U256> for TrimmedU256 {
    fn from(v: U256) -> Self {
        Self(v)
    }
}

impl From<TrimmedU256> for U256 {
    fn from(v: TrimmedU256) -> Self {
        v.0
    }
}

/// Trimmed storage slot value encoding with combined tag+length.
///
/// Encoding format:
/// - `0x00` = zero/deleted value (None)
/// - `0x01-0x20` = length of value, followed by that many big-endian bytes
///
/// This combines the "has value" tag with the length prefix for efficiency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrimmedStorageValue(pub Option<U256>);

impl Codec for TrimmedStorageValue {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        match self.0 {
            None => {
                enc.write_buf(&[0u8])?;
            }
            Some(v) if v.is_zero() => {
                enc.write_buf(&[0u8])?;
            }
            Some(v) => {
                let bytes = v.to_be_bytes::<32>();
                // Find first non-zero byte
                let start = bytes.iter().position(|&b| b != 0).unwrap_or(32);
                let len = (32 - start) as u8;
                enc.write_buf(&[len])?;
                enc.write_buf(&bytes[start..])?;
            }
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let [len] = dec.read_arr::<1>()?;
        let len = len as usize;

        if len == 0 {
            return Ok(Self(None));
        }

        if len > 32 {
            return Err(CodecError::MalformedField(
                "TrimmedStorageValue length exceeds 32",
            ));
        }

        let mut buf = [0u8; 32];
        dec.read_buf(&mut buf[32 - len..])?;
        Ok(Self(Some(U256::from_be_bytes(buf))))
    }
}

impl From<Option<U256>> for TrimmedStorageValue {
    fn from(v: Option<U256>) -> Self {
        Self(v)
    }
}

impl From<TrimmedStorageValue> for Option<U256> {
    fn from(v: TrimmedStorageValue) -> Self {
        v.0
    }
}

/// Wrapper for U256 that implements `Codec`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodecU256(pub U256);

impl Codec for CodecU256 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(&self.0.to_le_bytes::<32>())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let buf = dec.read_arr::<32>()?;
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodecB256(pub B256);

impl Codec for CodecB256 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(self.0.as_slice())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let buf = dec.read_arr::<32>()?;
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodecAddress(pub Address);

impl Codec for CodecAddress {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(self.0.as_slice())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let buf = dec.read_arr::<20>()?;
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

    #[test]
    fn test_trimmed_u256_zero() {
        let val = TrimmedU256(U256::ZERO);
        let encoded = encode_to_vec(&val).unwrap();
        // Zero should encode as just [0] (length = 0)
        assert_eq!(encoded, vec![0]);
        let decoded: TrimmedU256 = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }

    #[test]
    fn test_trimmed_u256_small() {
        let val = TrimmedU256(U256::from(0x42u8));
        let encoded = encode_to_vec(&val).unwrap();
        // Small value should encode as [1, 0x42] (1 byte)
        assert_eq!(encoded, vec![1, 0x42]);
        let decoded: TrimmedU256 = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }

    #[test]
    fn test_trimmed_u256_medium() {
        let val = TrimmedU256(U256::from(0x1234u16));
        let encoded = encode_to_vec(&val).unwrap();
        // 0x1234 needs 2 bytes
        assert_eq!(encoded, vec![2, 0x12, 0x34]);
        let decoded: TrimmedU256 = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }

    #[test]
    fn test_trimmed_u256_full() {
        // Create a value with all 32 bytes used (MSB is non-zero)
        let mut bytes = [0xffu8; 32];
        bytes[0] = 0x80; // MSB set
        let val = TrimmedU256(U256::from_be_bytes(bytes));
        let encoded = encode_to_vec(&val).unwrap();
        // Full 32-byte value: [32] + 32 bytes = 33 bytes total
        assert_eq!(encoded.len(), 33);
        assert_eq!(encoded[0], 32);
        let decoded: TrimmedU256 = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }

    #[test]
    fn test_trimmed_storage_value_none() {
        let val = TrimmedStorageValue(None);
        let encoded = encode_to_vec(&val).unwrap();
        // None encodes as [0]
        assert_eq!(encoded, vec![0]);
        let decoded: TrimmedStorageValue = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }

    #[test]
    fn test_trimmed_storage_value_zero() {
        // Some(zero) also encodes as [0] (same as None)
        let val = TrimmedStorageValue(Some(U256::ZERO));
        let encoded = encode_to_vec(&val).unwrap();
        assert_eq!(encoded, vec![0]);
        // Decodes back as None (which is semantically equivalent for storage)
        let decoded: TrimmedStorageValue = decode_buf_exact(&encoded).unwrap();
        assert_eq!(decoded.0, None);
    }

    #[test]
    fn test_trimmed_storage_value_small() {
        let val = TrimmedStorageValue(Some(U256::from(100u8)));
        let encoded = encode_to_vec(&val).unwrap();
        // [1, 100] - 1 byte value
        assert_eq!(encoded, vec![1, 100]);
        let decoded: TrimmedStorageValue = decode_buf_exact(&encoded).unwrap();
        assert_eq!(decoded.0, Some(U256::from(100u8)));
    }

    #[test]
    fn test_trimmed_storage_value_address_sized() {
        // Simulate an address stored in storage (20 bytes)
        let addr_bytes = [0x42u8; 20];
        let mut full = [0u8; 32];
        full[12..].copy_from_slice(&addr_bytes);
        let val = TrimmedStorageValue(Some(U256::from_be_bytes(full)));
        let encoded = encode_to_vec(&val).unwrap();
        // [20] + 20 bytes = 21 bytes total
        assert_eq!(encoded.len(), 21);
        assert_eq!(encoded[0], 20);
        let decoded: TrimmedStorageValue = decode_buf_exact(&encoded).unwrap();
        assert_eq!(val, decoded);
    }

    #[test]
    fn test_trimmed_encoding_savings() {
        // Demonstrate savings for storage values
        // Keys use fixed 32-byte encoding (most are keccak256 hashes)
        // Values use trimmed encoding for significant savings

        let value = U256::from(1000u16); // small counter
        let val_enc = encode_to_vec(&TrimmedStorageValue(Some(value))).unwrap();

        // Old value encoding: 1 (has_value) + 32 (value) = 33 bytes
        // New value encoding: 1 (len) + 2 (bytes) = 3 bytes
        // Savings: 30 bytes per slot with small values (91%)
        assert_eq!(val_enc.len(), 3);
        assert_eq!(val_enc, vec![2, 0x03, 0xe8]);

        // For comparison, TrimmedU256 would encode a small key efficiently,
        // but we don't use it for keys since most are hashes
        let small_key = U256::from(5u8);
        let key_trimmed = encode_to_vec(&TrimmedU256(small_key)).unwrap();
        assert_eq!(key_trimmed, vec![1, 5]); // Would be 2 bytes if we used trimming
                                             // But hash keys would be 33 bytes (1 + 32), worse than
                                             // fixed 32 bytes
    }
}
