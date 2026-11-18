//! Withdrawal message types for bridge gateway account communication.

use strata_codec::{BufDecoder, Codec, CodecError, Decoder, Encoder, VarVec};
use strata_identifiers::SubjectId;

/// Message type ID for withdrawal initiation.
pub const WITHDRAWAL_MSG_TYPE_ID: u16 = 0x03;

/// Message type ID for withdrawal fee bump.
pub const WITHDRAWAL_FEE_BUMP_MSG_TYPE_ID: u16 = 0x04;

/// Message type ID for withdrawal rejection.
pub const WITHDRAWAL_REJECTION_MSG_TYPE_ID: u16 = 0x05;

/// Maximum length for withdrawal destination descriptor.
pub const MAX_WITHDRAWAL_DESC_LEN: u32 = 255;

// TODO(STR-580): allow users to specify operator fee
pub const DEFAULT_OPERATOR_FEE: u32 = 0;

/// Bounded [`VarVec`] holding destination descriptor.
pub type DestDescBufVec = VarVec<u8, { MAX_WITHDRAWAL_DESC_LEN }>;

/// Message data for withdrawal initiation to the bridge gateway account.
///
/// This message type is sent by accounts that want to trigger a withdrawal.
/// The value sent with the message should be equal to the predetermined
/// static withdrawal size.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct WithdrawalMsgData {
    /// Fees in satoshis to be paid to the operator.
    ///
    /// Currently, this is just ignored.
    fees: u32,

    /// User's selected operator index for withdrawal assignment.
    selected_operator: u32,

    /// Bitcoin Output Script Descriptor describing the withdrawal output.
    dest_desc: DestDescBufVec,
}

impl WithdrawalMsgData {
    /// Creates a new withdrawal message data instance.
    pub fn new(fees: u32, dest_desc: Vec<u8>, selected_operator: u32) -> Option<Self> {
        // Ensure the destination descriptor isn't too long.
        if dest_desc.len() > MAX_WITHDRAWAL_DESC_LEN as usize {
            return None;
        }

        let dest_desc = VarVec::from_vec(dest_desc)?;
        Some(Self {
            fees,
            selected_operator,
            dest_desc,
        })
    }

    /// Decodes a withdrawal message body, accepting every layout of the
    /// descriptor's length prefix.
    ///
    /// The [`Codec`] decoding of this type accepts the length prefix only in
    /// the shortest layout that holds it, which is the one encoding writes.
    /// Until `strata-codec` v0.5.1 it accepted the one-byte, two-byte, and
    /// four-byte layouts for any length they hold, and the OL rules before V3
    /// are fixed to that behavior. This decodes as it did then, and returns
    /// the same message for a body [`Codec`] decoding accepts.
    ///
    /// # Errors
    ///
    /// Returns a [`CodecError`] if `body` ends early, has bytes after the
    /// descriptor, or holds a descriptor longer than
    /// [`MAX_WITHDRAWAL_DESC_LEN`].
    pub fn decode_with_any_len_layout(body: &[u8]) -> Result<Self, CodecError> {
        let mut dec = BufDecoder::new(body);
        let fees = u32::decode(&mut dec)?;
        let selected_operator = u32::decode(&mut dec)?;

        let dest_desc_len = decode_len_in_any_layout(&mut dec)?;
        if dest_desc_len > MAX_WITHDRAWAL_DESC_LEN {
            return Err(CodecError::OverflowContainer);
        }
        let mut dest_desc = vec![0; dest_desc_len as usize];
        dec.read_buf(&mut dest_desc)?;
        if dec.remaining() > 0 {
            return Err(CodecError::ExtraInput);
        }

        Self::new(fees, dest_desc, selected_operator).ok_or(CodecError::OverflowContainer)
    }

    /// Get the fees paid to the operator, in sats.
    pub fn fees(&self) -> u32 {
        self.fees
    }

    /// Get the destination descriptor as bytes.
    pub fn dest_desc(&self) -> &[u8] {
        self.dest_desc.as_ref()
    }

    /// Takes out the inner destination descriptor as a `VarVec`.
    pub fn into_dest_desc(self) -> DestDescBufVec {
        self.dest_desc
    }

    /// Gets the user's selected operator index.
    pub fn selected_operator(&self) -> u32 {
        self.selected_operator
    }
}

/// Decodes a `strata-codec` varint without requiring its shortest layout.
///
/// The top two bits of the first byte select the layout: one byte holding
/// seven bits, two bytes holding 14, or four bytes holding 30, big-endian.
/// This is the decoding of `strata-codec` before v0.5.1, which the layout
/// check of later versions narrows.
///
/// `strata-codec` owns the format and no longer offers this decoding, so the
/// two places that are fixed to it each carry it: this one, and the inbox
/// message length in `strata-ol-da-types-v1`. Neither crate depends on the
/// other, and both copy a decoding that never changes again.
fn decode_len_in_any_layout(dec: &mut impl Decoder) -> Result<u32, CodecError> {
    let first_byte = u8::decode(dec)?;
    match first_byte >> 6 {
        0 | 1 => Ok(u32::from(first_byte)),
        2 => {
            let second_byte = u8::decode(dec)?;
            Ok(u32::from(u16::from_be_bytes([
                first_byte & 0x3f,
                second_byte,
            ])))
        }
        _ => {
            let mut bytes = [first_byte & 0x3f, 0, 0, 0];
            dec.read_buf(&mut bytes[1..])?;
            Ok(u32::from_be_bytes(bytes))
        }
    }
}

/// Message data for withdrawal fee bump.
///
/// This message type is sent by accounts that want to bump the fee for
/// a pending withdrawal in the withdrawal intents queue.
///
/// This is currently unused and unsupported.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct WithdrawalFeeBumpMsgData {
    /// Index of the withdrawal intent to bump.
    withdrawal_intent_idx: u32,

    /// Source subject requesting the fee bump.
    source_subject: SubjectId,
}

impl WithdrawalFeeBumpMsgData {
    /// Create a new withdrawal fee bump message data instance.
    pub fn new(withdrawal_intent_idx: u32, source_subject: SubjectId) -> Self {
        Self {
            withdrawal_intent_idx,
            source_subject,
        }
    }

    /// Get the withdrawal intent index.
    pub fn withdrawal_intent_idx(&self) -> u32 {
        self.withdrawal_intent_idx
    }

    /// Get the source subject.
    pub fn source_subject(&self) -> &SubjectId {
        &self.source_subject
    }
}

/// Rejection type for withdrawal rejection messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WithdrawalRejectionType {
    /// The withdrawal was rejected entirely.
    RejectedEntirely = 0,

    /// This is a withdrawal fee bump rejection.
    FeeBumpRejection = 1,
}

impl WithdrawalRejectionType {
    /// Convert from u8 representation.
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            0 => Some(Self::RejectedEntirely),
            1 => Some(Self::FeeBumpRejection),
            _ => None,
        }
    }

    /// Convert to u8 representation.
    pub fn to_u8(self) -> u8 {
        self as u8
    }
}

impl Codec for WithdrawalRejectionType {
    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        Self::from_u8(dec.read_arr::<1>()?[0])
            .ok_or(CodecError::InvalidVariant("WithdrawalRejectionType"))
    }

    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        enc.write_buf(&[self.to_u8()])
    }
}

/// Message data for withdrawal rejection from the bridge gateway account.
///
/// This message type occurs when the bridge gateway account rejects
/// a withdrawal initiation or a withdrawal fee bump.
///
/// This is currently unused.
#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct WithdrawalRejectionMsgData {
    /// Index of the withdrawal intent.
    withdrawal_intent_idx: u32,

    /// Type of rejection.
    rejection_type: WithdrawalRejectionType,

    /// Source subject that initiated the withdrawal.
    source_subject: SubjectId,
}

impl WithdrawalRejectionMsgData {
    /// Create a new withdrawal rejection message data instance.
    pub fn new(
        withdrawal_intent_idx: u32,
        rejection_type: WithdrawalRejectionType,
        source_subject: SubjectId,
    ) -> Self {
        Self {
            withdrawal_intent_idx,
            rejection_type,
            source_subject,
        }
    }

    /// Get the withdrawal intent index.
    pub fn withdrawal_intent_idx(&self) -> u32 {
        self.withdrawal_intent_idx
    }

    /// Get the rejection type.
    pub fn rejection_type(&self) -> WithdrawalRejectionType {
        self.rejection_type
    }

    /// Get the source subject.
    pub fn source_subject(&self) -> &SubjectId {
        &self.source_subject
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use strata_codec::{decode_buf_exact, encode_to_vec};

    use super::*;

    proptest! {
        #[test]
        fn test_withdrawal_msg_data_codec(
            fees in 0u32..=u32::MAX,
            dest_desc_bytes in prop::collection::vec(any::<u8>(), 0..=255),
            selected_operator in any::<u32>(),
        ) {
            let msg_data = WithdrawalMsgData::new(fees, dest_desc_bytes, selected_operator)
                .expect("WithdrawalMsgData creation should succeed");

            // Encode
            let encoded = encode_to_vec(&msg_data).expect("Encoding should succeed");

            // Decode
            let decoded: WithdrawalMsgData = decode_buf_exact(&encoded)
                .expect("Decoding should succeed");

            // Verify round-trip
            prop_assert_eq!(decoded.fees, msg_data.fees);
            prop_assert_eq!(decoded.dest_desc.as_ref(), msg_data.dest_desc.as_ref());
            prop_assert_eq!(decoded.selected_operator, msg_data.selected_operator);
        }

        #[test]
        fn test_any_len_layout_decoding_matches_codec_on_encoded_bodies(
            fees in any::<u32>(),
            dest_desc_bytes in prop::collection::vec(any::<u8>(), 0..=255),
            selected_operator in any::<u32>(),
        ) {
            let msg_data = WithdrawalMsgData::new(fees, dest_desc_bytes, selected_operator)
                .expect("descriptor fits the bound");
            let encoded = encode_to_vec(&msg_data).expect("encoding succeeds");

            prop_assert_eq!(
                WithdrawalMsgData::decode_with_any_len_layout(&encoded).ok(),
                Some(msg_data)
            );
        }

        #[test]
        fn test_any_len_layout_decoding_accepts_what_codec_accepts(
            body in prop::collection::vec(any::<u8>(), 0..=300),
        ) {
            if let Ok(strict) = decode_buf_exact::<WithdrawalMsgData>(&body) {
                prop_assert_eq!(
                    WithdrawalMsgData::decode_with_any_len_layout(&body).ok(),
                    Some(strict)
                );
            }
        }
    }

    /// Builds a withdrawal body whose descriptor carries `len_prefix` in
    /// place of the length prefix encoding writes.
    fn body_with_len_prefix(len_prefix: &[u8], dest_desc: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&7u32.to_be_bytes());
        body.extend_from_slice(&3u32.to_be_bytes());
        body.extend_from_slice(len_prefix);
        body.extend_from_slice(dest_desc);
        body
    }

    #[test]
    fn test_any_len_layout_decoding_accepts_longer_layouts() {
        let dest_desc = [0xab; 22];
        let expected = WithdrawalMsgData::new(7, dest_desc.to_vec(), 3).expect("fits the bound");

        // The length 22 in the one-byte layout encoding writes, then in the
        // two-byte and four-byte layouts.
        let shortest = body_with_len_prefix(&[0x16], &dest_desc);
        assert_eq!(encode_to_vec(&expected).expect("encodes"), shortest);
        for len_prefix in [&[0x16][..], &[0x80, 0x16], &[0xc0, 0x00, 0x00, 0x16]] {
            let body = body_with_len_prefix(len_prefix, &dest_desc);
            assert_eq!(
                WithdrawalMsgData::decode_with_any_len_layout(&body).ok(),
                Some(expected.clone()),
                "length prefix {len_prefix:02x?}"
            );
        }

        // The codec decoding takes only the shortest layout.
        for len_prefix in [&[0x80, 0x16][..], &[0xc0, 0x00, 0x00, 0x16]] {
            let body = body_with_len_prefix(len_prefix, &dest_desc);
            assert!(
                decode_buf_exact::<WithdrawalMsgData>(&body).is_err(),
                "length prefix {len_prefix:02x?}"
            );
        }
    }

    #[test]
    fn test_any_len_layout_decoding_rejects_malformed_bodies() {
        // A descriptor one byte above the bound, in the two-byte layout.
        let too_long = body_with_len_prefix(&[0x81, 0x00], &[0; 256]);
        assert!(matches!(
            WithdrawalMsgData::decode_with_any_len_layout(&too_long),
            Err(CodecError::OverflowContainer)
        ));

        // Bytes after the descriptor.
        let trailing = body_with_len_prefix(&[0x80, 0x02], &[1, 2, 3]);
        assert!(matches!(
            WithdrawalMsgData::decode_with_any_len_layout(&trailing),
            Err(CodecError::ExtraInput)
        ));

        // A descriptor shorter than its length prefix, and a length prefix
        // that ends early.
        let short = body_with_len_prefix(&[0x80, 0x04], &[1, 2, 3]);
        assert!(WithdrawalMsgData::decode_with_any_len_layout(&short).is_err());
        let cut_prefix = body_with_len_prefix(&[0xc0, 0x00], &[]);
        assert!(WithdrawalMsgData::decode_with_any_len_layout(&cut_prefix).is_err());
    }

    #[test]
    fn test_withdrawal_rejection_type_conversion() {
        // This just test all values exhaustively.
        assert_eq!(
            WithdrawalRejectionType::from_u8(0),
            Some(WithdrawalRejectionType::RejectedEntirely)
        );
        assert_eq!(
            WithdrawalRejectionType::from_u8(1),
            Some(WithdrawalRejectionType::FeeBumpRejection)
        );
        assert_eq!(WithdrawalRejectionType::from_u8(2), None);

        assert_eq!(WithdrawalRejectionType::RejectedEntirely.to_u8(), 0);
        assert_eq!(WithdrawalRejectionType::FeeBumpRejection.to_u8(), 1);
    }
}
