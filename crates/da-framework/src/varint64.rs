//! Variable-length integer types covering the full u64 range.
//!
//! Provides compact LEB128-style encoding for both signed and unsigned values
//! where small values (the common case) use fewer bytes.
//!
//! `encode` is canonical: it writes a value in the fewest bytes that hold it
//! and never sets the sign bit on a zero magnitude.
//!
//! `decode` deliberately accepts more than `encode` writes, exactly what it
//! accepted at `v0.5.0-rc.2`: a final byte with no data bits (padding), an
//! eleventh byte with no data bits, a sign bit on a zero magnitude (read as
//! zero), and data bits above the 64th in the tenth byte (dropped). The frozen
//! checkpoint programs that prove V0 to V2 epochs decode DA payloads this way
//! and nothing binds the DA bytes to a canonical form, so a stricter decoder
//! here would fail on payloads those programs accept and stall nodes replaying
//! those epochs (SECFND-912). The V3 rules check canonical form for the whole
//! payload instead, by requiring that re-encoding the decoded payload
//! reproduces its bytes.
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

/// Decodes the magnitude bits after the first byte, accumulating into `value`
/// from bit position `start_bits`.
///
/// The body is the `v0.5.0-rc.2` decoder, kept byte for byte because the
/// frozen V0 to V2 checkpoint programs run it (SECFND-912). It accepts
/// noncanonical encodings and decodes them as those programs do:
///
/// - A final byte with no data bits is padding and adds nothing.
/// - The group at bit 63 (unsigned) or 62 (signed) is shifted in with a plain `<<`, which drops its
///   data bits above the 64th instead of rejecting them.
/// - One more group past the 64th bit is accepted if it carries no data bits, so an encoding may
///   run to eleven bytes.
///
/// Canonical form is checked by the V3 rules on the whole payload, not here.
///
/// # Errors
///
/// Rejects a group past the 64th bit that carries data bits, and a twelfth
/// byte.
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

    /// Decodes as `v0.5.0-rc.2` did, accepting `decode_magnitude_tail`'s
    /// noncanonical forms because the frozen V0 to V2 checkpoint programs
    /// accept them (SECFND-912). Canonical form is checked by the V3 rules on
    /// the whole payload.
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
/// zero, which `new` never produces and `decode` reads as zero.
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
        // decoder reads a sign bit on it as zero, so never write one.
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

    /// Decodes as `v0.5.0-rc.2` did, through `decode_magnitude_tail`'s
    /// noncanonical forms and with a sign bit on a zero magnitude normalized
    /// to zero by [`Self::new`] rather than rejected, because the frozen V0 to
    /// V2 checkpoint programs accept it (SECFND-912). Canonical form is
    /// checked by the V3 rules on the whole payload.
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

    /// Spellings `encode` never writes decode to the value they spell, as at
    /// `v0.5.0-rc.2`: a final byte with no data bits is padding.
    #[test]
    fn test_unsigned_varint_decodes_padded_encodings_to_their_value() {
        for (buf, value) in [
            (&[0x80, 0x00][..], 0),       // 0 in two bytes
            (&[0x81, 0x00][..], 1),       // 1 in two bytes
            (&[0xFF, 0x00][..], 127),     // 127 in two bytes
            (&[0x80, 0x80, 0x00][..], 0), // 0 in three bytes
        ] {
            let decoded = decode_buf_exact::<UnsignedVarInt>(buf)
                .unwrap_or_else(|e| panic!("{buf:02x?} must decode, got {e:?}"));
            assert_eq!(decoded, UnsignedVarInt::new(value), "{buf:02x?}");
        }
    }

    /// As for the unsigned form, and a sign bit on a zero magnitude decodes to
    /// the positive zero `new` builds.
    #[test]
    fn test_signed_varint_decodes_noncanonical_encodings_to_their_value() {
        for (buf, value) in [
            (&[0x40][..], SignedVarInt::ZERO),               // -0
            (&[0x80, 0x00][..], SignedVarInt::ZERO),         // +0 in two bytes
            (&[0xC0, 0x00][..], SignedVarInt::ZERO),         // -0 in two bytes
            (&[0xBF, 0x00][..], SignedVarInt::positive(63)), // 63 in two bytes
            (&[0xFF, 0x00][..], SignedVarInt::negative(63)), // -63 in two bytes
        ] {
            let decoded = decode_buf_exact::<SignedVarInt>(buf)
                .unwrap_or_else(|e| panic!("{buf:02x?} must decode, got {e:?}"));
            assert_eq!(decoded, value, "{buf:02x?}");
        }
    }

    /// Every well-formed one- and two-byte input decodes. Re-encoding writes
    /// the minimal spelling of the same value, which is the input itself
    /// exactly when the input is minimal: the rule the V3 payload check
    /// relies on.
    #[test]
    fn test_unsigned_varint_every_short_encoding_reencodes_minimally() {
        for buf in short_inputs() {
            let v = decode_buf_exact::<UnsignedVarInt>(&buf)
                .unwrap_or_else(|e| panic!("{buf:02x?} must decode, got {e:?}"));
            let reencoded = encode_to_vec(&v).unwrap();
            assert!(is_minimal(&reencoded, false), "{buf:02x?}");
            assert_eq!(decode_buf_exact::<UnsignedVarInt>(&reencoded).unwrap(), v);
            assert_eq!(reencoded == buf, is_minimal(&buf, false), "{buf:02x?}");
        }
    }

    #[test]
    fn test_signed_varint_every_short_encoding_reencodes_minimally() {
        for buf in short_inputs() {
            let v = decode_buf_exact::<SignedVarInt>(&buf)
                .unwrap_or_else(|e| panic!("{buf:02x?} must decode, got {e:?}"));
            let reencoded = encode_to_vec(&v).unwrap();
            assert!(is_minimal(&reencoded, true), "{buf:02x?}");
            assert_eq!(decode_buf_exact::<SignedVarInt>(&reencoded).unwrap(), v);
            assert_eq!(reencoded == buf, is_minimal(&buf, true), "{buf:02x?}");
        }
    }

    /// Ten-byte inputs at the top of the u64 range decode as `v0.5.0-rc.2`
    /// decodes them. The tenth group starts at bit 63 (unsigned) or 62
    /// (signed) and is shifted in with a plain `<<`, which drops its data bits
    /// above the 64th, so these alias values the encoder spells differently.
    #[test]
    fn test_varint64_drops_data_bits_above_the_64th_in_the_tenth_byte() {
        let nine_ff = [0xFFu8; MAX_VARINT_BYTES - 1];
        let mut max_canonical = nine_ff.to_vec();
        max_canonical.push(0x01);
        let mut max_alias = nine_ff.to_vec();
        max_alias.push(0x7F); // u64::MAX with junk above bit 63
        let mut two_pow_64 = [0x80u8; MAX_VARINT_BYTES - 1].to_vec();
        two_pow_64.push(0x02); // bit 64 alone

        for (buf, value) in [
            (&max_canonical[..], UnsignedVarInt::MAX),
            // 0x7F << 63 keeps only bit 63, so the value is u64::MAX.
            (&max_alias[..], UnsignedVarInt::MAX),
            // 0x02 << 63 shifts the one data bit out, so the value is 0.
            (&two_pow_64[..], UnsignedVarInt::ZERO),
        ] {
            let decoded = decode_buf_exact::<UnsignedVarInt>(buf)
                .unwrap_or_else(|e| panic!("unsigned {buf:02x?} must decode, got {e:?}"));
            assert_eq!(decoded, value, "unsigned {buf:02x?}");
        }
        assert_eq!(encode_to_vec(&UnsignedVarInt::MAX).unwrap(), max_canonical);

        // Signed: six bits in the first byte, so the tenth group holds two.
        let mut signed_canonical = nine_ff.to_vec();
        signed_canonical.push(0x03);
        let mut signed_alias = nine_ff.to_vec();
        signed_alias.push(0x07); // -u64::MAX with junk above bit 63
        let neg_max = SignedVarInt::negative(u64::MAX);
        // 0x07 << 62 keeps bits 62 and 63, so the magnitude is u64::MAX.
        for buf in [&signed_canonical, &signed_alias] {
            let decoded = decode_buf_exact::<SignedVarInt>(buf)
                .unwrap_or_else(|e| panic!("signed {buf:02x?} must decode, got {e:?}"));
            assert_eq!(decoded, neg_max, "signed {buf:02x?}");
        }
        assert_eq!(encode_to_vec(&neg_max).unwrap(), signed_canonical);
    }

    /// The decoder reads one group past the 64th bit, as `v0.5.0-rc.2` does:
    /// an eleventh byte with no data bits is accepted, one with data bits is
    /// overflow, and a twelfth byte is refused. The random-byte proptests
    /// rarely reach a well-formed input this long, so these cases are fixed.
    #[test]
    fn test_varint64_accepts_only_an_empty_eleventh_byte() {
        let mut eleven_zero = [0x80u8; MAX_VARINT_BYTES].to_vec();
        eleven_zero.push(0x00);
        let mut continued_max = [0xFFu8; MAX_VARINT_BYTES - 1].to_vec();
        continued_max.extend([0x81, 0x00]); // bit 63, then an empty group
        for (buf, value) in [
            (&eleven_zero, UnsignedVarInt::ZERO),
            (&continued_max, UnsignedVarInt::MAX),
        ] {
            let decoded = decode_buf_exact::<UnsignedVarInt>(buf)
                .unwrap_or_else(|e| panic!("unsigned {buf:02x?} must decode, got {e:?}"));
            assert_eq!(decoded, value, "unsigned {buf:02x?}");
        }

        let mut signed_continued_max = [0xFFu8; MAX_VARINT_BYTES - 1].to_vec();
        signed_continued_max.extend([0x83, 0x00]); // bits 62 and 63, then an empty group
        assert_eq!(
            decode_buf_exact::<SignedVarInt>(&signed_continued_max).unwrap(),
            SignedVarInt::negative(u64::MAX)
        );
        assert_eq!(
            decode_buf_exact::<SignedVarInt>(&eleven_zero).unwrap(),
            SignedVarInt::ZERO
        );

        let mut eleven_data = [0x80u8; MAX_VARINT_BYTES].to_vec();
        eleven_data.push(0x01);
        let mut twelve = [0x80u8; MAX_VARINT_BYTES + 1].to_vec();
        twelve.push(0x00);
        for (buf, reason) in [
            (&eleven_data, "varint64 overflow"),
            (&twelve, "varint64 too many bytes"),
        ] {
            let res = decode_buf_exact::<UnsignedVarInt>(buf);
            assert!(
                matches!(res, Err(CodecError::MalformedField(r)) if r == reason),
                "unsigned {buf:02x?} must be rejected with {reason:?}, got {res:?}"
            );
            let res = decode_buf_exact::<SignedVarInt>(buf);
            assert!(
                matches!(res, Err(CodecError::MalformedField(r)) if r == reason),
                "signed {buf:02x?} must be rejected with {reason:?}, got {res:?}"
            );
        }
    }

    /// `i64::MIN` is the one i64 whose magnitude, 2^63, needs the tenth byte,
    /// so its bytes are pinned rather than only round-tripped.
    #[test]
    fn test_signed_varint_i64_min_bytes() {
        let mut expected = vec![0xC0]; // continuation and sign, no data bits
        expected.extend([0x80; 8]); // bits 6..=61 are zero
        expected.push(0x02); // bit 63 is the second bit of the tenth group
        assert_eq!(expected.len(), MAX_VARINT_BYTES);

        let min = SignedVarInt::from_i64(i64::MIN);
        assert_eq!(encode_to_vec(&min).unwrap(), expected);
        assert_eq!(decode_buf_exact::<SignedVarInt>(&expected).unwrap(), min);
        assert_eq!(min.to_i64(), Some(i64::MIN));
    }

    proptest! {
        /// Any byte string that decodes re-encodes, in no more bytes, to a
        /// spelling of the same value. Whether the input was that spelling is
        /// for the V3 payload check, which compares the bytes.
        #[test]
        fn proptest_unsigned_varint_accepted_input_reencodes_to_the_same_value(
            buf in prop::collection::vec(any::<u8>(), 1..=MAX_VARINT_BYTES + 2)
        ) {
            if let Ok(v) = decode_buf_exact::<UnsignedVarInt>(&buf) {
                let reencoded = encode_to_vec(&v).unwrap();
                prop_assert!(reencoded.len() <= buf.len());
                prop_assert_eq!(decode_buf_exact::<UnsignedVarInt>(&reencoded).unwrap(), v);
            }
        }

        #[test]
        fn proptest_signed_varint_accepted_input_reencodes_to_the_same_value(
            buf in prop::collection::vec(any::<u8>(), 1..=MAX_VARINT_BYTES + 2)
        ) {
            if let Ok(v) = decode_buf_exact::<SignedVarInt>(&buf) {
                let reencoded = encode_to_vec(&v).unwrap();
                prop_assert!(reencoded.len() <= buf.len());
                prop_assert_eq!(decode_buf_exact::<SignedVarInt>(&reencoded).unwrap(), v);
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
