//! Deposit descriptor wire format used by bridge deposits.
//!
//! A descriptor says "route this deposit to account serial X and subject bytes Y". The encoding is
//! compact: it avoids zero padding in the subject and uses fewer bytes for small serial values.
//!
//! Layout (all values are big-endian):
//! `[control:1][serial:1..=3][subject:0..=32]`
//!
//! Control byte (bit 7 is MSB):
//! - bits 7..6: reserved, must be 0
//! - bits 5..4: serial length (00=1 byte, 01=2 bytes, 10=3 bytes, 11 reserved)
//! - bits 3..0: the 4 bits immediately above the encoded serial bytes
//!
//! Serial encoding:
//! - A serial is a `u32`, but only 28 bits are encodable (max `0x0FFF_FFFF`).
//! - Pick the smallest length that fits: `<= 0x0FFF` -> 1 byte, `<= 0x0FFFFF` -> 2 bytes, `<=
//!   0x0FFF_FFFF` -> 3 bytes.
//! - Write the least-significant `serial_len` bytes to the buffer; store the next 4 bits in
//!   `control[3..0]`.
//! - To decode, rebuild a 4-byte big-endian buffer, insert the serial bytes at the end, then OR in
//!   the nibble from `control[3..0]`.
//!
//! Subject encoding:
//! - The subject has no explicit length; it is "the rest of the bytes" after the serial.
//! - Length must be `<= 32`.
//! - When expanded to a 32-byte `SubjectId`, the bytes are left-padded with zeros.
//!
//! Because the subject length is implicit, callers must pass the exact descriptor slice (the
//! surrounding container must be length-delimited).

use arbitrary::Arbitrary;
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_identifiers::{AccountSerial, SUBJ_ID_LEN};
use thiserror::Error;

use super::SubjectIdBytes;

/// Minimum descriptor length: control byte + 1 serial byte, with empty subject bytes.
pub const MIN_DESCRIPTOR_LEN: usize = 1 + 1;

/// Maximum descriptor length: control byte + 3 serial bytes + max subject bytes.
pub const MAX_DESCRIPTOR_LEN: usize = 1 + 3 + SUBJ_ID_LEN;

/// Maximum serial value encodable by the descriptor format (28 bits).
pub const MAX_SERIAL_VALUE: u32 = (1 << 28) - 1;

const CONTROL_RESERVED_MASK: u8 = 0b1100_0000;
const CONTROL_LEN_BIT0: u8 = 0b0010_0000;
const CONTROL_LEN_BIT1: u8 = 0b0001_0000;
const CONTROL_SERIAL_MSB_MASK: u8 = 0b0000_1111;

const MAX_SERIAL_12_BITS: u32 = (1 << 12) - 1;
const MAX_SERIAL_20_BITS: u32 = (1 << 20) - 1;
const MAX_SERIAL_28_BITS: u32 = MAX_SERIAL_VALUE;

/// Errors for deposit descriptor parsing and encoding.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DepositDescriptorError {
    /// Descriptor is too short to contain the control byte and serial bytes.
    #[error("descriptor length {actual} is shorter than required {expected}")]
    InsufficientLength { expected: usize, actual: usize },

    /// Reserved control bits were set.
    #[error("reserved control bits set: {0:#04x}")]
    ReservedControlBits(u8),

    /// Reserved serial length bits were set.
    #[error("reserved serial length bits set: {0:#04x}")]
    ReservedSerialLengthBits(u8),

    /// Subject bytes exceed the maximum allowed length.
    #[error("subject bytes length {0} exceeds maximum length {1}")]
    SubjectTooLong(usize, usize),

    /// Serial value exceeds the maximum encodable range.
    #[error("serial {0} exceeds maximum encodable value {1}")]
    SerialTooLarge(u32, u32),
}

/// Deposit descriptor for routing bridge deposits.
///
/// This struct is the in-memory representation of the wire format described above. The stored
/// subject bytes are unpadded; use [`SubjectIdBytes::to_subject_id`] when you need a 32-byte
/// subject identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepositDescriptor {
    dest_acct_serial: AccountSerial,
    dest_subject: SubjectIdBytes,
}

impl DepositDescriptor {
    /// Creates a new deposit descriptor.
    pub fn new(dest_acct_serial: AccountSerial, dest_subject: SubjectIdBytes) -> Self {
        Self {
            dest_acct_serial,
            dest_subject,
        }
    }

    /// Returns a reference to destination account serial.
    pub const fn dest_acct_serial(&self) -> &AccountSerial {
        &self.dest_acct_serial
    }

    /// Returns a reference to destination subject bytes.
    pub const fn dest_subject(&self) -> &SubjectIdBytes {
        &self.dest_subject
    }

    /// Consumes the descriptor and returns its parts.
    pub fn into_parts(self) -> (AccountSerial, SubjectIdBytes) {
        (self.dest_acct_serial, self.dest_subject)
    }

    /// Encodes this descriptor into a byte vector.
    ///
    /// # Errors
    ///
    /// Returns an error if the account serial exceeds the maximum encodable range.
    pub fn encode_to_vec(&self) -> Result<Vec<u8>, DepositDescriptorError> {
        let serial = *self.dest_acct_serial.inner();
        let serial_len = Self::serial_len_for_value(serial)?;
        let control = Self::control_byte(serial, serial_len);
        let serial_bytes = Self::serial_bytes(serial, serial_len);

        let mut out = Vec::with_capacity(1 + serial_len + self.dest_subject.len());
        out.push(control);
        out.extend_from_slice(&serial_bytes);
        out.extend_from_slice(self.dest_subject.as_bytes());
        Ok(out)
    }

    /// Decodes a descriptor from a byte slice.
    pub fn decode_from_slice(bytes: &[u8]) -> Result<Self, DepositDescriptorError> {
        if bytes.len() < MIN_DESCRIPTOR_LEN {
            return Err(DepositDescriptorError::InsufficientLength {
                expected: MIN_DESCRIPTOR_LEN,
                actual: bytes.len(),
            });
        }

        let control = bytes[0];
        if control & CONTROL_RESERVED_MASK != 0 {
            return Err(DepositDescriptorError::ReservedControlBits(control));
        }

        let len_bits =
            (((control & CONTROL_LEN_BIT0) >> 5) << 1) | ((control & CONTROL_LEN_BIT1) >> 4);
        if len_bits == 3 {
            return Err(DepositDescriptorError::ReservedSerialLengthBits(control));
        }
        let serial_len = (len_bits as usize) + 1;
        let expected_len = 1 + serial_len;
        if bytes.len() < expected_len {
            return Err(DepositDescriptorError::InsufficientLength {
                expected: expected_len,
                actual: bytes.len(),
            });
        }

        let serial_bytes = &bytes[1..expected_len];
        let serial = Self::decode_serial(control, serial_len, serial_bytes);

        let subject_len = bytes.len() - expected_len;
        if subject_len > SUBJ_ID_LEN {
            return Err(DepositDescriptorError::SubjectTooLong(
                subject_len,
                SUBJ_ID_LEN,
            ));
        }
        let subject_bytes = bytes[expected_len..].to_vec();
        let dest_subject = SubjectIdBytes::try_new(subject_bytes).expect("length validated above");

        Ok(Self {
            dest_acct_serial: AccountSerial::new(serial),
            dest_subject,
        })
    }

    fn serial_len_for_value(serial: u32) -> Result<usize, DepositDescriptorError> {
        if serial <= MAX_SERIAL_12_BITS {
            Ok(1)
        } else if serial <= MAX_SERIAL_20_BITS {
            Ok(2)
        } else if serial <= MAX_SERIAL_28_BITS {
            Ok(3)
        } else {
            Err(DepositDescriptorError::SerialTooLarge(
                serial,
                MAX_SERIAL_VALUE,
            ))
        }
    }

    fn control_byte(serial: u32, serial_len: usize) -> u8 {
        let msb_nibble = ((serial >> (serial_len * 8)) & 0x0F) as u8;
        let len_bits = (serial_len - 1) as u8;
        let len_bit0 = (len_bits >> 1) & 0x01;
        let len_bit1 = len_bits & 0x01;
        (len_bit0 << 5) | (len_bit1 << 4) | msb_nibble
    }

    fn serial_bytes(serial: u32, serial_len: usize) -> Vec<u8> {
        let be = serial.to_be_bytes();
        be[(4 - serial_len)..].to_vec()
    }

    fn decode_serial(control: u8, serial_len: usize, serial_bytes: &[u8]) -> u32 {
        let mut serial_buf = [0u8; 4];
        serial_buf[(4 - serial_len)..].copy_from_slice(serial_bytes);
        let nibble_index = 3 - serial_len;
        serial_buf[nibble_index] |= control & CONTROL_SERIAL_MSB_MASK;
        u32::from_be_bytes(serial_buf)
    }
}

/// Manual `Codec` implementation because the descriptor has an implicit length.
///
/// WARNING: Decoding reads until EOF, so this only works when the descriptor is the final field
/// in its enclosing container. If it appears before other fields, decoding cannot know where to
/// stop.
impl Codec for DepositDescriptor {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        let encoded = self
            .encode_to_vec()
            .map_err(|_| CodecError::OverflowContainer)?;
        enc.write_buf(&encoded)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let mut bytes = Vec::new();
        while let Ok(byte) = dec.read_arr::<1>() {
            bytes.push(byte[0]);
        }
        Self::decode_from_slice(&bytes)
            .map_err(|_| CodecError::MalformedField("deposit descriptor"))
    }
}

impl<'a> Arbitrary<'a> for DepositDescriptor {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let serial = u.int_in_range(0..=MAX_SERIAL_VALUE)?;
        let dest_acct_serial = AccountSerial::new(serial);
        let dest_subject = u.arbitrary()?;
        Ok(Self::new(dest_acct_serial, dest_subject))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn subject_bytes() -> impl Strategy<Value = SubjectIdBytes> {
        prop::collection::vec(any::<u8>(), 0..=SUBJ_ID_LEN)
            .prop_map(|bytes| SubjectIdBytes::try_new(bytes).expect("length is within bounds"))
    }

    proptest! {
        #[test]
        fn roundtrip(
            serial in 0..=MAX_SERIAL_VALUE,
            subject in subject_bytes(),
        ) {
            let descriptor = DepositDescriptor::new(AccountSerial::new(serial), subject);
            let encoded = descriptor.encode_to_vec().expect("encoding should succeed");
            let decoded = DepositDescriptor::decode_from_slice(&encoded).expect("decode should succeed");
            prop_assert_eq!(decoded, descriptor);
        }

        #[test]
        fn encoded_len_matches_components(
            serial in 0..=MAX_SERIAL_VALUE,
            subject in subject_bytes(),
        ) {
            let descriptor = DepositDescriptor::new(AccountSerial::new(serial), subject.clone());
            let encoded = descriptor.encode_to_vec().expect("encoding should succeed");
            let expected_serial_len = DepositDescriptor::serial_len_for_value(serial).unwrap();
            prop_assert_eq!(encoded.len(), 1 + expected_serial_len + subject.len());
        }
    }

    #[test]
    fn encodes_expected_control_and_serial_bytes() {
        let subject = SubjectIdBytes::try_new(Vec::new()).expect("empty is valid");
        let cases = [
            (0x000_u32, 1_usize, 0x00_u8, vec![0x00]),
            (0x0FFF_u32, 1_usize, 0x0F_u8, vec![0xFF]),
            (0x1000_u32, 2_usize, 0x10_u8, vec![0x10, 0x00]),
            (0xFFFFF_u32, 2_usize, 0x1F_u8, vec![0xFF, 0xFF]),
            (0x100000_u32, 3_usize, 0x20_u8, vec![0x10, 0x00, 0x00]),
            (0x0FFFFFFF_u32, 3_usize, 0x2F_u8, vec![0xFF, 0xFF, 0xFF]),
        ];

        for (serial, serial_len, control, serial_bytes) in cases {
            let descriptor = DepositDescriptor::new(AccountSerial::new(serial), subject.clone());
            let encoded = descriptor.encode_to_vec().expect("encoding should succeed");
            assert_eq!(encoded[0], control);
            assert_eq!(&encoded[1..(1 + serial_len)], serial_bytes.as_slice());
            assert_eq!(encoded.len(), 1 + serial_len);
        }
    }

    #[test]
    fn encode_rejects_too_large_serial() {
        let subject = SubjectIdBytes::try_new(Vec::new()).expect("empty is valid");
        let serial = MAX_SERIAL_VALUE + 1;
        let descriptor = DepositDescriptor::new(AccountSerial::new(serial), subject);
        let err = descriptor.encode_to_vec().unwrap_err();
        assert_eq!(
            err,
            DepositDescriptorError::SerialTooLarge(serial, MAX_SERIAL_VALUE)
        );
    }

    #[test]
    fn decode_rejects_reserved_control_bits() {
        let bytes = [0b1000_0000_u8, 0x00];
        let err = DepositDescriptor::decode_from_slice(&bytes).unwrap_err();
        assert!(matches!(
            err,
            DepositDescriptorError::ReservedControlBits(_)
        ));
    }

    #[test]
    fn decode_rejects_reserved_length_bits() {
        let bytes = [0b0011_0000_u8, 0x00];
        let err = DepositDescriptor::decode_from_slice(&bytes).unwrap_err();
        assert!(matches!(
            err,
            DepositDescriptorError::ReservedSerialLengthBits(_)
        ));
    }

    #[test]
    fn decode_rejects_insufficient_length() {
        let bytes = [0b0000_0000_u8];
        let err = DepositDescriptor::decode_from_slice(&bytes).unwrap_err();
        assert!(matches!(
            err,
            DepositDescriptorError::InsufficientLength { .. }
        ));
    }

    #[test]
    fn decode_rejects_subject_too_long() {
        let mut bytes = vec![0b0000_0000_u8, 0x00];
        bytes.extend(std::iter::repeat_n(0u8, SUBJ_ID_LEN + 1));
        let err = DepositDescriptor::decode_from_slice(&bytes).unwrap_err();
        assert!(matches!(err, DepositDescriptorError::SubjectTooLong(_, _)));
    }
}
