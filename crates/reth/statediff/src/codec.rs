//! Codec wrappers for Alloy types.

use alloy_primitives::U256;
use revm_primitives::{Address, B256};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::CounterScheme;

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

/// Signed U256 delta for balance changes.
///
/// Encoding format:
/// - `0x00` = zero delta (not normally encoded, handled by DaCounter)
/// - `0x01` + TrimmedU256 = positive delta (balance increased)
/// - `0x02` + TrimmedU256 = negative delta (balance decreased)
///
/// This allows encoding balance changes of any magnitude up to U256::MAX,
/// using only 2-34 bytes depending on the delta size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignedU256Delta {
    /// True if this is a positive delta (addition), false for negative (subtraction).
    positive: bool,
    /// Absolute value of the change.
    magnitude: U256,
}

impl Default for SignedU256Delta {
    fn default() -> Self {
        // Zero delta is normalized to positive
        Self {
            positive: true,
            magnitude: U256::ZERO,
        }
    }
}

impl SignedU256Delta {
    /// Creates a new signed delta.
    fn new(positive: bool, magnitude: U256) -> Self {
        // Normalize: if magnitude is zero, sign doesn't matter
        if magnitude.is_zero() {
            Self {
                positive: true,
                magnitude: U256::ZERO,
            }
        } else {
            Self {
                positive,
                magnitude,
            }
        }
    }

    /// Creates a positive delta (balance increase).
    pub fn positive(magnitude: U256) -> Self {
        Self::new(true, magnitude)
    }

    /// Creates a negative delta (balance decrease).
    pub fn negative(magnitude: U256) -> Self {
        Self::new(false, magnitude)
    }

    /// Returns true if this delta is zero.
    pub fn is_zero(&self) -> bool {
        self.magnitude.is_zero()
    }

    /// Returns the sign (true = positive, false = negative).
    pub fn is_positive(&self) -> bool {
        self.positive
    }

    /// Returns the absolute magnitude of the change.
    pub fn magnitude(&self) -> U256 {
        self.magnitude
    }
}

impl Codec for SignedU256Delta {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        if self.magnitude.is_zero() {
            // Zero delta: single byte
            enc.write_buf(&[0x00])?;
        } else if self.positive {
            // Positive delta: 0x01 + trimmed magnitude
            enc.write_buf(&[0x01])?;
            TrimmedU256(self.magnitude).encode(enc)?;
        } else {
            // Negative delta: 0x02 + trimmed magnitude
            enc.write_buf(&[0x02])?;
            TrimmedU256(self.magnitude).encode(enc)?;
        }
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let [tag] = dec.read_arr::<1>()?;
        match tag {
            0x00 => Ok(Self::default()),
            0x01 => {
                let magnitude = TrimmedU256::decode(dec)?.0;
                Ok(Self::positive(magnitude))
            }
            0x02 => {
                let magnitude = TrimmedU256::decode(dec)?.0;
                Ok(Self::negative(magnitude))
            }
            _ => Err(CodecError::InvalidVariant("SignedU256Delta")),
        }
    }
}

/// Counter scheme for U256 balance with signed delta.
///
/// Supports balance changes of any magnitude (up to U256::MAX) in either direction.
/// Encoding is compact: 2-34 bytes depending on delta size.
#[derive(Clone, Copy, Debug, Default)]
pub struct CtrU256BySignedU256;

impl CounterScheme for CtrU256BySignedU256 {
    type Base = U256;
    type Incr = SignedU256Delta;

    fn is_zero(incr: &Self::Incr) -> bool {
        incr.is_zero()
    }

    fn update(base: &mut Self::Base, incr: &Self::Incr) {
        if incr.is_zero() {
            return;
        }
        if incr.positive {
            // Saturating add to prevent overflow
            *base = base.saturating_add(incr.magnitude);
        } else {
            // Saturating sub to prevent underflow
            *base = base.saturating_sub(incr.magnitude);
        }
    }

    fn compare(a: Self::Base, b: Self::Base) -> Option<Self::Incr> {
        // Compute b - a as a signed delta
        if b >= a {
            Some(SignedU256Delta::positive(b - a))
        } else {
            Some(SignedU256Delta::negative(a - b))
        }
    }
}

#[cfg(test)]
mod tests {
    use strata_codec::{decode_buf_exact, encode_to_vec};
    use strata_da_framework::{ContextlessDaWrite, CounterScheme, DaCounter};

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

    #[test]
    fn test_signed_u256_delta_zero() {
        let delta = SignedU256Delta::default();
        assert!(delta.is_zero());
        assert!(delta.is_positive()); // Zero is normalized to positive

        let encoded = encode_to_vec(&delta).unwrap();
        assert_eq!(encoded, vec![0x00]); // Single byte for zero

        let decoded: SignedU256Delta = decode_buf_exact(&encoded).unwrap();
        assert!(decoded.is_zero());
    }

    #[test]
    fn test_signed_u256_delta_positive_small() {
        let delta = SignedU256Delta::positive(U256::from(100u8));
        assert!(!delta.is_zero());
        assert!(delta.is_positive());
        assert_eq!(delta.magnitude(), U256::from(100u8));

        let encoded = encode_to_vec(&delta).unwrap();
        // 0x01 (positive tag) + [1, 100] (trimmed U256)
        assert_eq!(encoded, vec![0x01, 1, 100]);

        let decoded: SignedU256Delta = decode_buf_exact(&encoded).unwrap();
        assert_eq!(decoded.magnitude(), U256::from(100u8));
        assert!(decoded.is_positive());
    }

    #[test]
    fn test_signed_u256_delta_negative_small() {
        let delta = SignedU256Delta::negative(U256::from(50u8));
        assert!(!delta.is_zero());
        assert!(!delta.is_positive());
        assert_eq!(delta.magnitude(), U256::from(50u8));

        let encoded = encode_to_vec(&delta).unwrap();
        // 0x02 (negative tag) + [1, 50] (trimmed U256)
        assert_eq!(encoded, vec![0x02, 1, 50]);

        let decoded: SignedU256Delta = decode_buf_exact(&encoded).unwrap();
        assert_eq!(decoded.magnitude(), U256::from(50u8));
        assert!(!decoded.is_positive());
    }

    #[test]
    fn test_signed_u256_delta_large() {
        // Test with a large U256 value (full 32 bytes)
        let mut bytes = [0xffu8; 32];
        bytes[0] = 0x80;
        let large = U256::from_be_bytes(bytes);

        let delta = SignedU256Delta::positive(large);
        let encoded = encode_to_vec(&delta).unwrap();
        // 0x01 + [32] + 32 bytes = 34 bytes total
        assert_eq!(encoded.len(), 34);
        assert_eq!(encoded[0], 0x01);

        let decoded: SignedU256Delta = decode_buf_exact(&encoded).unwrap();
        assert_eq!(decoded.magnitude(), large);
        assert!(decoded.is_positive());
    }

    #[test]
    fn test_signed_u256_delta_normalization() {
        // Creating a "negative zero" should normalize to positive zero
        let delta = SignedU256Delta::new(false, U256::ZERO);
        assert!(delta.is_zero());
        assert!(delta.is_positive()); // Normalized
    }

    // ==================== CtrU256BySignedU256 Tests ====================

    #[test]
    fn test_ctr_u256_compare_increase() {
        let a = U256::from(100u8);
        let b = U256::from(150u8);

        let delta = CtrU256BySignedU256::compare(a, b).unwrap();
        assert!(delta.is_positive());
        assert_eq!(delta.magnitude(), U256::from(50u8));
    }

    #[test]
    fn test_ctr_u256_compare_decrease() {
        let a = U256::from(150u8);
        let b = U256::from(100u8);

        let delta = CtrU256BySignedU256::compare(a, b).unwrap();
        assert!(!delta.is_positive());
        assert_eq!(delta.magnitude(), U256::from(50u8));
    }

    #[test]
    fn test_ctr_u256_compare_no_change() {
        let a = U256::from(100u8);
        let b = U256::from(100u8);

        let delta = CtrU256BySignedU256::compare(a, b).unwrap();
        assert!(delta.is_zero());
    }

    #[test]
    fn test_ctr_u256_update_positive() {
        let mut base = U256::from(100u8);
        let delta = SignedU256Delta::positive(U256::from(50u8));

        CtrU256BySignedU256::update(&mut base, &delta);
        assert_eq!(base, U256::from(150u8));
    }

    #[test]
    fn test_ctr_u256_update_negative() {
        let mut base = U256::from(100u8);
        let delta = SignedU256Delta::negative(U256::from(30u8));

        CtrU256BySignedU256::update(&mut base, &delta);
        assert_eq!(base, U256::from(70u8));
    }

    #[test]
    fn test_ctr_u256_update_saturation() {
        // Test underflow saturation
        let mut base = U256::from(50u8);
        let delta = SignedU256Delta::negative(U256::from(100u8));

        CtrU256BySignedU256::update(&mut base, &delta);
        assert_eq!(base, U256::ZERO); // Saturates at zero

        // Test overflow saturation
        let mut base = U256::MAX;
        let delta = SignedU256Delta::positive(U256::from(100u8));

        CtrU256BySignedU256::update(&mut base, &delta);
        assert_eq!(base, U256::MAX); // Saturates at max
    }

    #[test]
    fn test_ctr_u256_da_counter_apply() {
        let delta = SignedU256Delta::negative(U256::from(25u8));
        let ctr = DaCounter::<CtrU256BySignedU256>::new_changed(delta);

        let mut balance = U256::from(100u8);
        ContextlessDaWrite::apply(&ctr, &mut balance).unwrap();

        assert_eq!(balance, U256::from(75u8));
    }

    #[test]
    fn test_ctr_u256_da_counter_unchanged() {
        let ctr = DaCounter::<CtrU256BySignedU256>::new_unchanged();
        assert!(!ctr.is_changed());

        let mut balance = U256::from(100u8);
        ContextlessDaWrite::apply(&ctr, &mut balance).unwrap();
        assert_eq!(balance, U256::from(100u8)); // Unchanged
    }

    #[test]
    fn test_ctr_u256_da_counter_zero_normalized() {
        // Creating a counter with zero delta should normalize to unchanged
        let delta = SignedU256Delta::positive(U256::ZERO);
        let ctr = DaCounter::<CtrU256BySignedU256>::new_changed(delta);
        assert!(!ctr.is_changed()); // Normalized to unchanged
    }

    #[test]
    fn test_ctr_u256_encoding_size_comparison() {
        // Compare encoding sizes: register vs counter for balance changes

        // Scenario: balance changes from 1 ETH to 1.001 ETH (small delta)
        let one_eth = U256::from(1_000_000_000_000_000_000u64); // 1e18 wei
        let small_delta = U256::from(1_000_000_000_000_000u64); // 0.001 ETH

        // Register encoding (full value): 32 bytes
        let register_enc = encode_to_vec(&CodecU256(one_eth + small_delta)).unwrap();
        assert_eq!(register_enc.len(), 32);

        // Counter encoding (delta only): 1 (tag) + 1 (len) + 8 (bytes) = 10 bytes
        let delta = SignedU256Delta::positive(small_delta);
        let counter_enc = encode_to_vec(&delta).unwrap();
        assert!(counter_enc.len() < register_enc.len());

        // For very small deltas, savings are even better
        let tiny_delta = U256::from(100u8);
        let tiny_enc = encode_to_vec(&SignedU256Delta::positive(tiny_delta)).unwrap();
        // 1 (tag) + 1 (len) + 1 (byte) = 3 bytes vs 32 bytes
        assert_eq!(tiny_enc.len(), 3);
    }
}
