//! Variable-length integer types covering the full u64 range.
//!
//! Provides compact LEB128-style encoding for both signed and unsigned values
//! where small values (the common case) use fewer bytes.
//!
//! The encoding is canonical: a value is written in the fewest bytes that hold
//! it, and decoding rejects anything longer (a final byte with no data bits
//! set, or a sign bit on a zero magnitude). Every value therefore has exactly
//! one byte representation, so bytes covered by a hash or a signature cannot be
//! rewritten without changing what they decode to.
//!
//! TODO(STR-2148): Eventually move to strata-codec and reconcile with its VarInt type.

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
///
/// # Errors
///
/// Rejects a group that starts past the 64th bit (ten bytes already hold a
/// u64, so an eleventh could only be padding or overflow), a group with data
/// bits above the 64th, and a final byte with no data bits set:
/// [`encode_magnitude_tail`] stops as soon as the magnitude is exhausted, so
/// a zero last byte is padding that gives the same value a second
/// representation. Together these leave every value exactly one encoding.
///
/// `start_bits` is the number of bits the first byte held, between 1 and 63;
/// the overflow check shifts by `64 - shift` and needs it below 64.
fn decode_magnitude_tail(
    mut value: u64,
    start_bits: u32,
    dec: &mut impl Decoder,
) -> Result<u64, CodecError> {
    debug_assert!(
        (1..64).contains(&start_bits),
        "varint64: first byte holds 1..=63 bits"
    );
    let mut shift = start_bits;
    loop {
        if shift >= 64 {
            return Err(CodecError::MalformedField("varint64 too many bytes"));
        }
        let [byte] = dec.read_arr::<1>()?;
        let data = (byte & 0x7F) as u64;
        // The group must fit the bits a u64 has left at this position; a
        // plain `<<` would drop the excess and make the value ambiguous.
        if data >> (64 - shift) != 0 {
            return Err(CodecError::MalformedField("varint64 overflow"));
        }
        value |= data << shift;
        if byte & 0x80 == 0 {
            if data == 0 {
                return Err(CodecError::MalformedField("varint64 not minimal"));
            }
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
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SignedVarInt {
    positive: bool,
    magnitude: u64,
}

/// Zero, as [`SignedVarInt::ZERO`]: the derived default would be a negative
/// zero, which `new` never produces and `decode` refuses.
impl Default for SignedVarInt {
    fn default() -> Self {
        Self::ZERO
    }
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
        // Zero has no sign: the constructors already normalize it, and the
        // decoder refuses a sign bit on it, so never write one.
        let sign_bit: u8 = if self.positive || mag == 0 { 0 } else { 0x40 };

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

        // `new` normalizes a zero magnitude to positive and `encode` never sets
        // the sign bit on one, so a sign bit here is a second spelling of zero.
        if !positive && magnitude == 0 {
            return Err(CodecError::MalformedField("varint64 negative zero"));
        }

        // With negative zero refused the fields are already normalized, so
        // they are taken as decoded rather than through `new`.
        Ok(Self {
            positive,
            magnitude,
        })
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{boundary_u64_strategy, decode_buf_exact, encode_to_vec};

    const MAX_VARINT_BYTES: usize = 10;

    fn signed_varint_strategy() -> impl Strategy<Value = SignedVarInt> {
        (
            prop_oneof![Just(true), Just(false)],
            boundary_u64_strategy(),
        )
            .prop_map(|(positive, magnitude)| SignedVarInt::new(positive, magnitude))
    }

    /// The canonical form by definition: a well-formed byte string is minimal
    /// iff its last byte carries data (or is the lone first byte) and, for the
    /// signed form, a zero magnitude carries no sign bit.
    fn is_minimal(bytes: &[u8], signed: bool) -> bool {
        let last = *bytes.last().unwrap();
        if bytes.len() > 1 && last & 0x7F == 0 {
            return false;
        }
        // A multi-byte zero already failed above, so the sign-bit rule only
        // has the single byte to cover.
        !(signed && bytes.len() == 1 && bytes[0] & 0x7F == 0x40)
    }

    /// Every well-formed one- and two-byte input: continuation bits consistent
    /// with the length.
    fn short_inputs() -> impl Iterator<Item = Vec<u8>> {
        let one = (0x00u8..=0x7F).map(|b| vec![b]);
        let two = (0x80u8..=0xFF).flat_map(|a| (0x00u8..=0x7F).map(move |b| vec![a, b]));
        one.chain(two)
    }

    #[test]
    fn test_unsigned_varint_rejects_noncanonical_encodings() {
        for buf in [
            &[0x80, 0x00][..],       // 0 in two bytes
            &[0x81, 0x00][..],       // 1 in two bytes
            &[0xFF, 0x00][..],       // 127 in two bytes
            &[0x80, 0x80, 0x00][..], // 0 in three bytes
        ] {
            let res = decode_buf_exact::<UnsignedVarInt>(buf);
            assert!(
                matches!(res, Err(CodecError::MalformedField("varint64 not minimal"))),
                "{buf:02x?} must be rejected, got {res:?}"
            );
        }
    }

    #[test]
    fn test_signed_varint_rejects_noncanonical_encodings() {
        for (buf, reason) in [
            (&[0x40][..], "varint64 negative zero"),     // -0
            (&[0x80, 0x00][..], "varint64 not minimal"), // +0 in two bytes
            (&[0xC0, 0x00][..], "varint64 not minimal"), // -0 in two bytes
            (&[0xBF, 0x00][..], "varint64 not minimal"), // 63 in two bytes
        ] {
            let res = decode_buf_exact::<SignedVarInt>(buf);
            assert!(
                matches!(res, Err(CodecError::MalformedField(r)) if r == reason),
                "{buf:02x?} must be rejected with {reason:?}, got {res:?}"
            );
        }
    }

    #[test]
    fn test_unsigned_varint_every_accepted_short_encoding_reencodes_to_itself() {
        for buf in short_inputs() {
            match decode_buf_exact::<UnsignedVarInt>(&buf) {
                Ok(v) => {
                    assert!(
                        is_minimal(&buf, false),
                        "{buf:02x?} accepted but not minimal"
                    );
                    assert_eq!(encode_to_vec(&v).unwrap(), buf, "{buf:02x?}");
                }
                Err(CodecError::MalformedField(_)) => {
                    assert!(!is_minimal(&buf, false), "{buf:02x?} rejected but minimal")
                }
                Err(e) => panic!("{buf:02x?}: unexpected error {e:?}"),
            }
        }
    }

    #[test]
    fn test_signed_varint_every_accepted_short_encoding_reencodes_to_itself() {
        for buf in short_inputs() {
            match decode_buf_exact::<SignedVarInt>(&buf) {
                Ok(v) => {
                    assert!(
                        is_minimal(&buf, true),
                        "{buf:02x?} accepted but not minimal"
                    );
                    assert_eq!(encode_to_vec(&v).unwrap(), buf, "{buf:02x?}");
                }
                Err(CodecError::MalformedField(_)) => {
                    assert!(!is_minimal(&buf, true), "{buf:02x?} rejected but minimal")
                }
                Err(e) => panic!("{buf:02x?}: unexpected error {e:?}"),
            }
        }
    }

    /// Ten-byte inputs at the top of the u64 range: the last group holds the
    /// one (unsigned) or two (signed) bits a u64 has left, so any higher bit
    /// set there is overflow, which a plain shift used to drop silently.
    #[test]
    fn test_varint64_rejects_overflow_and_padding_in_the_last_group() {
        let nine_ff = [0xFFu8; MAX_VARINT_BYTES - 1];
        let mut max_alias = nine_ff.to_vec();
        max_alias.push(0x7F); // u64::MAX with junk above bit 63
        let mut two_pow_64 = [0x80u8; MAX_VARINT_BYTES - 1].to_vec();
        two_pow_64.push(0x02); // bit 64
        let mut eleven = [0x80u8; MAX_VARINT_BYTES].to_vec();
        eleven.push(0x00); // one group past the maximum
        let mut max_canonical = nine_ff.to_vec();
        max_canonical.push(0x01);

        for (buf, reason) in [
            (&max_alias[..], "varint64 overflow"),
            (&two_pow_64[..], "varint64 overflow"),
            (&eleven[..], "varint64 too many bytes"),
        ] {
            let res = decode_buf_exact::<UnsignedVarInt>(buf);
            assert!(
                matches!(res, Err(CodecError::MalformedField(r)) if r == reason),
                "unsigned {buf:02x?} must be rejected with {reason:?}, got {res:?}"
            );
        }
        let max: UnsignedVarInt = decode_buf_exact(&max_canonical).unwrap();
        assert_eq!(max, UnsignedVarInt::MAX);
        assert_eq!(encode_to_vec(&max).unwrap(), max_canonical);

        // Signed: six bits in the first byte, so the tenth group holds two.
        let mut signed_alias = nine_ff.to_vec();
        signed_alias.push(0x07); // -u64::MAX with junk above bit 63
        let mut signed_canonical = nine_ff.to_vec();
        signed_canonical.push(0x03);
        let res = decode_buf_exact::<SignedVarInt>(&signed_alias);
        assert!(
            matches!(res, Err(CodecError::MalformedField("varint64 overflow"))),
            "signed {signed_alias:02x?} must be rejected, got {res:?}"
        );
        let neg_max: SignedVarInt = decode_buf_exact(&signed_canonical).unwrap();
        assert_eq!(neg_max, SignedVarInt::negative(u64::MAX));
        assert_eq!(encode_to_vec(&neg_max).unwrap(), signed_canonical);
    }

    proptest! {
        /// Any byte string that decodes re-encodes to exactly itself: the
        /// property the fix exists for, over arbitrary inputs up to one byte
        /// past the longest valid encoding.
        #[test]
        fn proptest_unsigned_varint_accepted_input_is_canonical(
            buf in prop::collection::vec(any::<u8>(), 1..=MAX_VARINT_BYTES + 1)
        ) {
            if let Ok(v) = decode_buf_exact::<UnsignedVarInt>(&buf) {
                prop_assert_eq!(encode_to_vec(&v).unwrap(), buf);
            }
        }

        #[test]
        fn proptest_signed_varint_accepted_input_is_canonical(
            buf in prop::collection::vec(any::<u8>(), 1..=MAX_VARINT_BYTES + 1)
        ) {
            if let Ok(v) = decode_buf_exact::<SignedVarInt>(&buf) {
                prop_assert_eq!(encode_to_vec(&v).unwrap(), buf);
            }
        }
    }

    #[test]
    fn test_signed_varint_default_is_zero_and_round_trips() {
        let d = SignedVarInt::default();
        assert_eq!(d, SignedVarInt::ZERO);
        assert!(d.is_positive());
        let encoded = encode_to_vec(&d).unwrap();
        assert_eq!(encoded, vec![0x00]);
        assert_eq!(decode_buf_exact::<SignedVarInt>(&encoded).unwrap(), d);
    }

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

    proptest! {
        #[test]
        fn proptest_unsigned_varint_codec_roundtrip(value in boundary_u64_strategy()) {
            let varint = UnsignedVarInt::new(value);

            let encoded = encode_to_vec(&varint).expect("test: encode unsigned varint");
            let decoded: UnsignedVarInt = decode_buf_exact(&encoded).expect("test: decode unsigned varint");

            prop_assert_eq!(decoded, varint);
        }

        #[test]
        fn proptest_signed_varint_codec_roundtrip(value in signed_varint_strategy()) {
            let encoded = encode_to_vec(&value).expect("test: encode signed varint");
            let decoded: SignedVarInt = decode_buf_exact(&encoded).expect("test: decode signed varint");

            prop_assert_eq!(decoded, value);
            if decoded.is_zero() {
                prop_assert!(decoded.is_positive());
            }
        }

        #[test]
        fn proptest_signed_varint_zero_is_normalized(positive in any::<bool>()) {
            let value = SignedVarInt::new(positive, 0);

            prop_assert_eq!(value, SignedVarInt::ZERO);
            prop_assert!(value.is_positive());
        }
    }
}
