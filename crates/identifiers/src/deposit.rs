//! Deposit descriptor wire format used by bridge deposits.
//!
//! A descriptor says "route this deposit to account serial X and subject bytes Y". The encoding
//! uses a compact wire format optimized for Bitcoin data availability costs by avoiding zero
//! padding and using variable-length encoding.

use arbitrary::Arbitrary;
use strata_codec::VarVec;
use thiserror::Error;

use crate::{AccountSerial, SubjectIdBytes, acct::SubjectIdBytesError};

/// Maximum value encodable in 12-bit serial format (4,095).
const MAX_SERIAL_12_BITS: u32 = (1 << 12) - 1;

/// Maximum value encodable in 20-bit serial format (1,048,575).
const MAX_SERIAL_20_BITS: u32 = (1 << 20) - 1;

/// Maximum value encodable in 28-bit serial format (268,435,455).
const MAX_SERIAL_28_BITS: u32 = (1 << 28) - 1;

/// Maximum serial value encodable by the descriptor format (28 bits = 268,435,455).
const MAX_SERIAL_VALUE: u32 = MAX_SERIAL_28_BITS;

/// Control byte in the deposit descriptor wire format.
///
/// Encodes both the length of the serial number encoding (1-3 bytes) and the
/// most significant nibble of the serial value itself.
///
/// # Layout
///
/// Bit positions (bit 7 is MSB):
/// ```text
/// [R R L L N N N N]
///  7 6 5 4 3 2 1 0
/// ```
///
/// - `R` (bits 7-6): Reserved (must be 0)
/// - `L` (bits 5-4): Serial length encoding (00=1 byte, 01=2 bytes, 10=3 bytes, 11=reserved)
/// - `N` (bits 3-0): MSB nibble (upper 4 bits) of the serial value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ControlByte(u8);

impl ControlByte {
    const RESERVED_MASK: u8 = 0b1100_0000;
    const LEN_MASK: u8 = 0b0011_0000;
    const MSB_NIBBLE_MASK: u8 = 0b0000_1111;

    /// Returns the raw byte value.
    fn as_u8(self) -> u8 {
        self.0
    }

    fn serial_len(self) -> usize {
        let len_bits = (self.0 & Self::LEN_MASK) >> 4;
        (len_bits as usize) + 1
    }

    fn serial_msb(self) -> u8 {
        self.0 & Self::MSB_NIBBLE_MASK
    }

    fn set_serial_len(&mut self, len: u8) {
        self.0 |= len << 4
    }

    fn set_serial_msb(&mut self, msb: u8) {
        self.0 |= msb
    }
}

impl TryFrom<u8> for ControlByte {
    type Error = DepositDescriptorError;

    /// Validates and creates a control byte from a raw byte value.
    ///
    /// # Errors
    ///
    /// Returns an error if reserved bits are set or if reserved length encoding is used.
    fn try_from(byte: u8) -> Result<Self, Self::Error> {
        if byte & Self::RESERVED_MASK != 0 {
            return Err(DepositDescriptorError::ReservedControlBits(byte));
        }

        let len_bits = (byte & Self::LEN_MASK) >> 4;
        if len_bits == 3 {
            return Err(DepositDescriptorError::ReservedSerialLengthBits(byte));
        }

        Ok(Self(byte))
    }
}

/// Intermediate representation of a serialized account serial.
///
/// Combines a control byte with additional payload bytes to represent
/// a variable-length encoding of an account serial number (12-28 bits).
///
/// This representation makes the encoding logic clearer by working directly
/// with the big-endian byte representation.
struct EncodedSerial {
    /// Control byte containing length encoding and MSB nibble.
    control: ControlByte,

    /// Additional 1-3 bytes containing the remaining serial bits.
    additional_bytes: Vec<u8>,
}

impl EncodedSerial {
    /// Encodes an account serial into variable-length wire format.
    ///
    /// Returns [`DepositDescriptorError::SerialTooLarge`] if the serial exceeds
    /// the maximum 28-bit value (268,435,455).
    fn from_account_serial(serial: AccountSerial) -> Result<Self, DepositDescriptorError> {
        let value = *serial.inner();
        let mut control = ControlByte::default();

        // Big-endian bytes: [msb, _, _, lsb]
        let [byte_0, byte_1, byte_2, byte_3] = value.to_be_bytes();

        let additional_bytes = if value <= MAX_SERIAL_12_BITS {
            control.set_serial_len(0);
            control.set_serial_msb(byte_2);
            vec![byte_3]
        } else if value <= MAX_SERIAL_20_BITS {
            control.set_serial_len(1);
            control.set_serial_msb(byte_1);
            vec![byte_2, byte_3]
        } else if value <= MAX_SERIAL_28_BITS {
            control.set_serial_len(2);
            control.set_serial_msb(byte_0);
            vec![byte_1, byte_2, byte_3]
        } else {
            return Err(DepositDescriptorError::SerialTooLarge(
                value,
                MAX_SERIAL_VALUE,
            ));
        };

        debug_assert_eq!(control.serial_len(), additional_bytes.len());

        Ok(Self {
            control,
            additional_bytes,
        })
    }

    /// Reconstructs the original account serial from the encoded representation.
    fn to_account_serial(&self) -> AccountSerial {
        let msb_nibble = self.control.serial_msb();
        let additional_len = self.additional_bytes.len();

        // Reconstruct the 4-byte big-endian representation
        let mut u32_bytes = [0u8; 4];

        // Copy the additional bytes to the end
        u32_bytes[(4 - additional_len)..].copy_from_slice(&self.additional_bytes);

        // OR in the MSB nibble at the appropriate position
        let msb_index = 4 - additional_len - 1;
        u32_bytes[msb_index] |= msb_nibble;

        AccountSerial::new(u32::from_be_bytes(u32_bytes))
    }

    /// Returns the control byte.
    fn control(&self) -> ControlByte {
        self.control
    }

    /// Returns the additional bytes.
    fn additional_bytes(&self) -> &[u8] {
        &self.additional_bytes
    }
}

/// Errors for deposit descriptor parsing and encoding.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DepositDescriptorError {
    /// Descriptor is empty.
    #[error("descriptor is empty")]
    EmptyDescriptor,

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
    #[error("invalid subject: {0}")]
    InvalidSubjec(#[from] SubjectIdBytesError),

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
    ///
    /// # Errors
    ///
    /// Returns [`DepositDescriptorError::SerialTooLarge`] if the account serial
    /// exceeds the maximum encodable value (268,435,455).
    pub fn new(
        dest_acct_serial: AccountSerial,
        dest_subject: SubjectIdBytes,
    ) -> Result<Self, DepositDescriptorError> {
        let value = *dest_acct_serial.inner();
        if value > MAX_SERIAL_VALUE {
            return Err(DepositDescriptorError::SerialTooLarge(
                value,
                MAX_SERIAL_VALUE,
            ));
        }
        Ok(Self {
            dest_acct_serial,
            dest_subject,
        })
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

    /// Encodes this descriptor into its compact wire format.
    ///
    /// This method cannot fail because the serial is validated in [`Self::new`].
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let encoded_serial = EncodedSerial::from_account_serial(self.dest_acct_serial)
            .expect("serial is valid by construction");

        let mut out = Vec::with_capacity(
            1 + encoded_serial.additional_bytes().len() + self.dest_subject.len(),
        );
        out.push(encoded_serial.control().as_u8());
        out.extend_from_slice(encoded_serial.additional_bytes());
        out.extend_from_slice(self.dest_subject.as_bytes());
        out
    }

    /// Convenience wrapper around [`Self::encode_to_vec`] that wraps the result in a [`VarVec`].
    pub fn encode_to_varvec(&self) -> VarVec<u8> {
        let encoded = self.encode_to_vec();
        VarVec::from_vec(encoded)
            .expect("descriptor length (max 36 bytes) is always within VARINT_MAX bound")
    }

    /// Decodes a descriptor from its wire format.
    pub fn decode_from_slice(bytes: &[u8]) -> Result<Self, DepositDescriptorError> {
        if bytes.is_empty() {
            return Err(DepositDescriptorError::EmptyDescriptor);
        }

        let control = ControlByte::try_from(bytes[0])?;
        let serial_len = control.serial_len();

        if bytes.len() < 1 + serial_len {
            return Err(DepositDescriptorError::InsufficientLength {
                expected: 1 + serial_len,
                actual: bytes.len(),
            });
        }

        // Reconstruct the account serial from control byte + additional bytes
        let serial_bytes = bytes[1..(1 + serial_len)].to_vec();
        let encoded = EncodedSerial {
            control,
            additional_bytes: serial_bytes,
        };
        let dest_acct_serial = encoded.to_account_serial();

        // Remaining bytes form the subject
        let subject_start = 1 + serial_len;
        let subject_bytes = bytes[subject_start..].to_vec();
        let dest_subject = SubjectIdBytes::try_new(subject_bytes)?;

        Ok(Self {
            dest_acct_serial,
            dest_subject,
        })
    }
}

impl<'a> Arbitrary<'a> for DepositDescriptor {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let serial = u.int_in_range(0..=MAX_SERIAL_VALUE)?;
        let dest_acct_serial = AccountSerial::new(serial);
        let dest_subject = u.arbitrary()?;
        Ok(Self::new(dest_acct_serial, dest_subject)
            .expect("serial is within valid range by construction"))
    }
}

#[cfg(test)]
mod tests {
    use std::iter::repeat_n;

    use proptest::prelude::*;

    use super::*;
    use crate::SUBJ_ID_LEN;

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
            let descriptor = DepositDescriptor::new(AccountSerial::new(serial), subject)
                .expect("serial is within valid range");
            let encoded = descriptor.encode_to_vec();
            let decoded = DepositDescriptor::decode_from_slice(&encoded).expect("decode should succeed");
            prop_assert_eq!(decoded, descriptor);
        }

        #[test]
        fn encoded_serial_roundtrip(serial in 0..=MAX_SERIAL_VALUE) {
            let account_serial = AccountSerial::new(serial);
            let encoded = EncodedSerial::from_account_serial(account_serial)
                .expect("encoding should succeed");
            let decoded = encoded.to_account_serial();
            prop_assert_eq!(decoded, account_serial);
        }
    }

    #[test]
    fn new_rejects_too_large_serial() {
        let subject = SubjectIdBytes::try_new(Vec::new()).expect("empty is valid");
        let serial = MAX_SERIAL_VALUE + 1;
        let err = DepositDescriptor::new(AccountSerial::new(serial), subject).unwrap_err();
        assert_eq!(
            err,
            DepositDescriptorError::SerialTooLarge(serial, MAX_SERIAL_VALUE)
        );
    }

    #[test]
    fn control_byte_rejects_reserved_control_bits() {
        let invalid_bytes = [
            0b1000_0000_u8, // Reserved bit 7 set
            0b0100_0000_u8, // Reserved bit 6 set
            0b1100_0000_u8, // Both reserved bits set
        ];

        for byte in invalid_bytes {
            let err = ControlByte::try_from(byte).unwrap_err();
            assert!(
                matches!(err, DepositDescriptorError::ReservedControlBits(_)),
                "Expected ReservedControlBits error for byte {:#010b}",
                byte
            );
        }
    }

    #[test]
    fn control_byte_rejects_reserved_length_bits() {
        let byte = 0b0011_0000_u8; // len_bits = 3 (reserved)
        let err = ControlByte::try_from(byte).unwrap_err();
        assert!(matches!(
            err,
            DepositDescriptorError::ReservedSerialLengthBits(_)
        ));
    }

    #[test]
    fn decode_rejects_empty_descriptor() {
        let bytes = [];
        let err = DepositDescriptor::decode_from_slice(&bytes).unwrap_err();
        assert!(matches!(err, DepositDescriptorError::EmptyDescriptor));
    }

    #[test]
    fn decode_rejects_truncated_serial() {
        let bytes = [0b0000_0000_u8]; // Control byte indicates 1 byte serial, but no serial bytes
        let err = DepositDescriptor::decode_from_slice(&bytes).unwrap_err();
        assert!(matches!(
            err,
            DepositDescriptorError::InsufficientLength { .. }
        ));
    }

    #[test]
    fn decode_rejects_subject_too_long() {
        let mut bytes = vec![0b0000_0000_u8, 0x00];
        bytes.extend(repeat_n(0u8, SUBJ_ID_LEN + 1));
        let err = DepositDescriptor::decode_from_slice(&bytes).unwrap_err();
        assert!(matches!(err, DepositDescriptorError::InvalidSubjec(_)));
    }
}
