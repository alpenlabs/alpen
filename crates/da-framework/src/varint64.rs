//! Variable-length integer types covering the full u64 range.
//!
//! Provides compact LEB128-style encoding for both signed and unsigned values
//! where small values (the common case) use fewer bytes.
//! 
//! TODO: Eventually move to strata-codec and reconcile with its VarInt type.

use crate::{Codec, CodecError, Decoder, Encoder};

/// Encodes remaining magnitude bits after the first byte has consumed `start_bits` bits.
///
/// Each subsequent byte uses 7 data bits with a continuation bit.
fn encode_magnitude_tail(
    mut magnitude: u64,
    start_bits: u32,
    enc: &mut impl Encoder,
) -> Result<(), CodecError> {
    magnitude >>= start_bits;
    while magnitude > 0 {
        let byte = if magnitude > 0x7F {
            0x80 | (magnitude & 0x7F) as u8 // continuation bit set
        } else {
            (magnitude & 0x7F) as u8 // last byte
        };
        enc.write_buf(&[byte])?;
        magnitude >>= 7;
    }
    Ok(())
}

/// Decodes remaining magnitude bits, accumulating into `first_data` starting at
/// bit position `start_bits`.
fn decode_magnitude_tail(
    mut value: u64,
    start_bits: u32,
    dec: &mut impl Decoder,
) -> Result<u64, CodecError> {
    let mut shift = start_bits;
    loop {
        if shift >= 64 + 7 {
            return Err(CodecError::MalformedField("varint64 too many bytes"));
        }
        let [byte] = dec.read_arr::<1>()?;
        let data = (byte & 0x7F) as u64;
        // Guard against shift overflow: only accumulate bits that fit in u64.
        if shift < 64 {
            value |= data << shift;
        } else if data != 0 {
            // Bits beyond 64 must be zero for a valid u64.
            return Err(CodecError::MalformedField("varint64 overflow"));
        }
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(value)
}

/// Unsigned variable-length integer covering full u64 range.
///
/// Encoding: LEB128 — each byte uses 7 data bits with a continuation bit.
/// Maximum 10 bytes for u64::MAX.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnsignedVarInt(u64);

impl UnsignedVarInt {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u64::MAX);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn inner(self) -> u64 {
        self.0
    }

    pub const fn is_zero(&self) -> bool {
        self.0 == 0
    }
}

impl Codec for UnsignedVarInt {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let value = self.0;
        // First byte: [C][D6..D0] — 7 data bits
        let first_data = (value & 0x7F) as u8;
        if value < 0x80 {
            // Fits in first byte, no continuation
            enc.write_buf(&[first_data])?;
            return Ok(());
        }

        // Continuation bit set
        enc.write_buf(&[0x80 | first_data])?;

        // Remaining value after first 7 bits, encoded 7 bits per byte
        encode_magnitude_tail(value, 7, enc)
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let [first] = dec.read_arr::<1>()?;
        let value = (first & 0x7F) as u64;
        if first & 0x80 == 0 {
            return Ok(Self(value));
        }
        let value = decode_magnitude_tail(value, 7, dec)?;
        Ok(Self(value))
    }
}


/// Signed variable-length integer covering full ±u64 magnitude range.
///
/// First byte: `[continuation][sign][6 data bits]`.
/// Subsequent bytes: `[continuation][7 data bits]`.
/// Maximum 10 bytes for full u64 magnitude.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SignedVarInt {
    positive: bool,
    magnitude: u64,
}

impl SignedVarInt {
    pub const ZERO: Self = Self {
        positive: true,
        magnitude: 0,
    };

    /// Creates a new signed varint. Zero magnitude normalizes to positive.
    pub fn new(positive: bool, magnitude: u64) -> Self {
        if magnitude == 0 {
            Self::ZERO
        } else {
            Self {
                positive,
                magnitude,
            }
        }
    }

    /// Creates from an i64. Values outside i64 range need [`Self::new`].
    pub fn from_i64(value: i64) -> Self {
        if value >= 0 {
            Self {
                positive: true,
                magnitude: value as u64,
            }
        } else {
            Self {
                positive: false,
                magnitude: value.unsigned_abs(),
            }
        }
    }

    /// Creates a positive value.
    pub fn positive(magnitude: u64) -> Self {
        Self::new(true, magnitude)
    }

    /// Creates a negative value. Zero magnitude normalizes to positive.
    pub fn negative(magnitude: u64) -> Self {
        Self::new(false, magnitude)
    }

    pub fn is_zero(&self) -> bool {
        self.magnitude == 0
    }

    pub fn is_positive(&self) -> bool {
        self.positive
    }

    pub fn is_negative(&self) -> bool {
        !self.positive && self.magnitude != 0
    }

    pub fn magnitude(&self) -> u64 {
        self.magnitude
    }

    /// Converts to i64 if the magnitude fits.
    pub fn to_i64(&self) -> Option<i64> {
        if self.magnitude == 0 {
            return Some(0);
        }
        if self.positive {
            i64::try_from(self.magnitude).ok()
        } else {
            // i64::MIN has magnitude i64::MAX + 1 = 2^63
            let min_mag = (i64::MAX as u64) + 1;
            if self.magnitude == min_mag {
                Some(i64::MIN)
            } else if self.magnitude <= i64::MAX as u64 {
                Some(-(self.magnitude as i64))
            } else {
                None
            }
        }
    }
}

impl Codec for SignedVarInt {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let mag = self.magnitude;
        let sign_bit: u8 = if self.positive { 0 } else { 0x40 };

        // First byte: [C][S][D5..D0] — 6 data bits
        let first_data = (mag & 0x3F) as u8;
        if mag < 0x40 {
            // Fits in first byte, no continuation
            enc.write_buf(&[sign_bit | first_data])?;
            return Ok(());
        }

        // Continuation bit set
        let first_byte = 0x80 | sign_bit | first_data;
        enc.write_buf(&[first_byte])?;

        // Remaining magnitude after first 6 bits, encoded 7 bits per byte
        encode_magnitude_tail(mag, 6, enc)
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let [first] = dec.read_arr::<1>()?;

        let positive = (first & 0x40) == 0;
        let first_data = (first & 0x3F) as u64;

        let magnitude = if first & 0x80 == 0 {
            // No continuation
            first_data
        } else {
            decode_magnitude_tail(first_data, 6, dec)?
        };

        Ok(Self::new(positive, magnitude))
    }
}

#[cfg(test)]
mod tests {
    use crate::{decode_buf_exact, encode_to_vec};

    use super::*;

    const MAX_VARINT_BYTES: usize = 10;

    // UnsignedVarInt tests

    #[test]
    fn test_unsigned_varint_zero() {
        let v = UnsignedVarInt::ZERO;
        let encoded = encode_to_vec(&v).unwrap();
        assert_eq!(encoded, vec![0x00]);
        let decoded: UnsignedVarInt = decode_buf_exact(&encoded).unwrap();
        assert_eq!(decoded.inner(), 0);
    }

    #[test]
    fn test_unsigned_varint_small() {
        for val in 0u64..=127 {
            let v = UnsignedVarInt::new(val);
            let encoded = encode_to_vec(&v).unwrap();
            assert_eq!(encoded.len(), 1);
            let decoded: UnsignedVarInt = decode_buf_exact(&encoded).unwrap();
            assert_eq!(decoded.inner(), val);
        }
    }

    #[test]
    fn test_unsigned_varint_boundaries() {
        let cases = [
            (127, 1),
            (128, 2),
            (16383, 2),
            (16384, 3),
            (u64::MAX, MAX_VARINT_BYTES),
        ];
        for (val, expected_len) in cases {
            let v = UnsignedVarInt::new(val);
            let encoded = encode_to_vec(&v).unwrap();
            assert_eq!(
                encoded.len(),
                expected_len,
                "value {} expected {} bytes, got {}",
                val,
                expected_len,
                encoded.len()
            );
            let decoded: UnsignedVarInt = decode_buf_exact(&encoded).unwrap();
            assert_eq!(decoded.inner(), val);
        }
    }

    #[test]
    fn test_unsigned_varint_encoding_examples() {
        // From spec
        assert_eq!(encode_to_vec(&UnsignedVarInt::new(0)).unwrap(), vec![0x00]);
        assert_eq!(encode_to_vec(&UnsignedVarInt::new(5)).unwrap(), vec![0x05]);
        assert_eq!(
            encode_to_vec(&UnsignedVarInt::new(127)).unwrap(),
            vec![0x7F]
        );
        assert_eq!(
            encode_to_vec(&UnsignedVarInt::new(128)).unwrap(),
            vec![0x80, 0x01]
        );
        assert_eq!(
            encode_to_vec(&UnsignedVarInt::new(255)).unwrap(),
            vec![0xFF, 0x01]
        );
        assert_eq!(
            encode_to_vec(&UnsignedVarInt::new(1000)).unwrap(),
            vec![0xE8, 0x07]
        );
    }

    #[test]
    fn test_unsigned_varint_roundtrip_exhaustive() {
        let test_values = [
            0,
            1,
            63,
            64,
            127,
            128,
            255,
            256,
            1000,
            16383,
            16384,
            u32::MAX as u64,
            u64::MAX / 2,
            u64::MAX,
        ];
        for val in test_values {
            let v = UnsignedVarInt::new(val);
            let encoded = encode_to_vec(&v).unwrap();
            let decoded: UnsignedVarInt = decode_buf_exact(&encoded).unwrap();
            assert_eq!(decoded.inner(), val, "roundtrip failed for {}", val);
        }
    }

    // SignedVarInt tests

    #[test]
    fn test_signed_varint_zero() {
        let v = SignedVarInt::ZERO;
        let encoded = encode_to_vec(&v).unwrap();
        assert_eq!(encoded, vec![0x00]);
        let decoded: SignedVarInt = decode_buf_exact(&encoded).unwrap();
        assert!(decoded.is_zero());
        assert!(decoded.is_positive()); // zero normalized to positive
    }

    #[test]
    fn test_signed_varint_small_positive() {
        for val in 0u64..=63 {
            let v = SignedVarInt::positive(val);
            let encoded = encode_to_vec(&v).unwrap();
            assert_eq!(encoded.len(), 1, "positive {} should be 1 byte", val);
            let decoded: SignedVarInt = decode_buf_exact(&encoded).unwrap();
            assert_eq!(decoded.magnitude(), val);
            assert!(decoded.is_positive() || val == 0);
        }
    }

    #[test]
    fn test_signed_varint_small_negative() {
        for val in 1u64..=63 {
            let v = SignedVarInt::negative(val);
            let encoded = encode_to_vec(&v).unwrap();
            assert_eq!(encoded.len(), 1, "negative {} should be 1 byte", val);
            let decoded: SignedVarInt = decode_buf_exact(&encoded).unwrap();
            assert!(!decoded.is_positive());
            assert_eq!(decoded.magnitude(), val);
        }
    }

    #[test]
    fn test_signed_varint_encoding_examples() {
        // From spec
        // +0: 0b0_0_000000 = 0x00
        assert_eq!(
            encode_to_vec(&SignedVarInt::positive(0)).unwrap(),
            vec![0x00]
        );
        // +5: 0b0_0_000101 = 0x05
        assert_eq!(
            encode_to_vec(&SignedVarInt::positive(5)).unwrap(),
            vec![0x05]
        );
        // -5: 0b0_1_000101 = 0x45
        assert_eq!(
            encode_to_vec(&SignedVarInt::negative(5)).unwrap(),
            vec![0x45]
        );
        // +100: 0b1_0_100100, 0b0_0000001 = 0xA4, 0x01
        assert_eq!(
            encode_to_vec(&SignedVarInt::positive(100)).unwrap(),
            vec![0xA4, 0x01]
        );
        // -100: 0b1_1_100100, 0b0_0000001 = 0xE4, 0x01
        assert_eq!(
            encode_to_vec(&SignedVarInt::negative(100)).unwrap(),
            vec![0xE4, 0x01]
        );
    }

    #[test]
    fn test_signed_varint_roundtrip_exhaustive() {
        let test_values = [
            0,
            1,
            63,
            64,
            127,
            128,
            8191,
            8192,
            u32::MAX as u64,
            u64::MAX / 2,
            u64::MAX,
        ];
        for mag in test_values {
            for positive in [true, false] {
                if mag == 0 && !positive {
                    continue; // skip negative zero
                }
                let v = SignedVarInt::new(positive, mag);
                let encoded = encode_to_vec(&v).unwrap();
                let decoded: SignedVarInt = decode_buf_exact(&encoded).unwrap();
                assert_eq!(decoded.magnitude(), mag);
                assert_eq!(decoded.is_positive(), positive || mag == 0);
            }
        }
    }

    #[test]
    fn test_signed_varint_negative_zero_normalization() {
        let v = SignedVarInt::negative(0);
        assert!(v.is_positive()); // normalized
        assert!(v.is_zero());
    }

    #[test]
    fn test_signed_varint_from_i64() {
        let v = SignedVarInt::from_i64(42);
        assert!(v.is_positive());
        assert_eq!(v.magnitude(), 42);

        let v = SignedVarInt::from_i64(-42);
        assert!(v.is_negative());
        assert_eq!(v.magnitude(), 42);

        let v = SignedVarInt::from_i64(0);
        assert!(v.is_zero());
    }

    #[test]
    fn test_signed_varint_to_i64() {
        assert_eq!(SignedVarInt::positive(42).to_i64(), Some(42));
        assert_eq!(SignedVarInt::negative(42).to_i64(), Some(-42));
        assert_eq!(SignedVarInt::ZERO.to_i64(), Some(0));
        assert_eq!(
            SignedVarInt::positive(i64::MAX as u64).to_i64(),
            Some(i64::MAX)
        );
        assert_eq!(
            SignedVarInt::negative(i64::MAX as u64 + 1).to_i64(),
            Some(i64::MIN)
        );
        // Beyond i64 range
        assert_eq!(SignedVarInt::positive(u64::MAX).to_i64(), None);
    }

    #[test]
    fn test_signed_varint_boundary_sizes() {
        let cases = [
            (63, 1),
            (64, 2),
            (8191, 2),
            (8192, 3),
            (u64::MAX, MAX_VARINT_BYTES),
        ];
        for (mag, expected_len) in cases {
            let pos = SignedVarInt::positive(mag);
            let neg = SignedVarInt::negative(mag);
            let enc_pos = encode_to_vec(&pos).unwrap();
            let enc_neg = encode_to_vec(&neg).unwrap();
            assert_eq!(
                enc_pos.len(),
                expected_len,
                "+{} expected {} bytes",
                mag,
                expected_len
            );
            assert_eq!(
                enc_neg.len(),
                expected_len,
                "-{} expected {} bytes",
                mag,
                expected_len
            );
        }
    }
}
