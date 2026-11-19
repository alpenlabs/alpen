//! Inbox accumulator types for snark accounts.

use strata_acct_types::{AccountId, BitcoinAmount, MsgPayload};
use strata_codec::{Codec, CodecError, Decoder, Encoder};
use strata_da_framework::LinearAccumulator;

use super::MAX_MSG_PAYLOAD_BYTES;

/// DA-encoded snark inbox message entry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaMessageEntryV1 {
    /// Account ID of the source account for the message.
    pub source: AccountId,

    /// Epoch in which the message was included.
    pub incl_epoch: u32,

    /// Message payload.
    pub payload: MsgPayload,
}

impl DaMessageEntryV1 {
    pub fn new(source: AccountId, incl_epoch: u32, payload: MsgPayload) -> Self {
        Self {
            source,
            incl_epoch,
            payload,
        }
    }
}

impl Codec for DaMessageEntryV1 {
    fn encode(&self, enc: &mut impl Encoder) -> Result<(), CodecError> {
        if self.payload.data().len() > MAX_MSG_PAYLOAD_BYTES {
            return Err(CodecError::OverflowContainer);
        }

        self.source.encode(enc)?;
        self.incl_epoch.encode(enc)?;
        self.payload.encode(enc)?;
        Ok(())
    }

    fn decode(dec: &mut impl Decoder) -> Result<Self, CodecError> {
        let source = AccountId::decode(dec)?;
        let incl_epoch = u32::decode(dec)?;
        let payload = decode_msg_payload(dec)?;
        Ok(Self {
            source,
            incl_epoch,
            payload,
        })
    }
}

/// Decodes an inbox message payload, accepting exactly the encodings
/// `v0.5.0-rc.2` accepted.
///
/// The layout is [`MsgPayload`]'s codec layout: the value, a length prefix, then
/// that many data bytes. At `v0.5.0-rc.2` the entry decoded it with
/// [`MsgPayload`]'s codec, whose `strata-codec` v0.4.0 length prefix accepted
/// any of its widths, and then refused data over [`MAX_MSG_PAYLOAD_BYTES`].
/// `strata-codec` v0.5.1 refuses a prefix wider than its value needs, so the
/// codec now refuses payloads the frozen V0 to V2 checkpoint programs accept.
/// This keeps their accept set and decoded values (SECFND-912). Canonical form
/// is checked by the V3 rules on the whole payload.
///
/// The length is checked against [`MAX_MSG_PAYLOAD_BYTES`] before the data
/// buffer is allocated. `v0.5.0-rc.2` read the data first and checked after,
/// so an over-long payload fails with a different error here, but the same
/// inputs fail.
fn decode_msg_payload(dec: &mut impl Decoder) -> Result<MsgPayload, CodecError> {
    let value = BitcoinAmount::decode(dec)?;
    let len = decode_any_width_len_prefix(dec)? as usize;
    if len > MAX_MSG_PAYLOAD_BYTES {
        return Err(CodecError::OverflowContainer);
    }

    let mut data = vec![0; len];
    dec.read_buf(&mut data)?;
    // The length is within the SSZ bound, which is far above the DA bound.
    MsgPayload::from_bytes(value, data).map_err(|_| CodecError::OverflowContainer)
}

/// Decodes a `strata-codec` varint length prefix in any of its widths, as
/// `strata-codec` v0.4.0 did, without requiring the shortest one.
///
/// The top two bits of the first byte choose the width: `0b0x` is one byte
/// holding the value, `0b10` two bytes and `0b11` four bytes, big-endian, with
/// those two bits masked out.
///
/// `strata-codec` owns the format and no longer offers this decoding.
/// `strata-ol-msg-types` carries the same one for the withdrawal descriptor
/// length. Neither crate depends on the other, and both copy a decoding that
/// never changes again.
fn decode_any_width_len_prefix(dec: &mut impl Decoder) -> Result<u32, CodecError> {
    let [first] = dec.read_arr::<1>()?;
    let value = match first >> 6 {
        0 | 1 => first as u32,
        2 => {
            let [second] = dec.read_arr::<1>()?;
            u16::from_be_bytes([first & 0x3f, second]) as u32
        }
        _ => {
            let [b1, b2, b3] = dec.read_arr::<3>()?;
            u32::from_be_bytes([first & 0x3f, b1, b2, b3])
        }
    };
    Ok(value)
}

/// Buffer of DA-encoded inbox messages for insertion into the real accumulator.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InboxBufferV1 {
    /// Inbox entries appended during the epoch.
    entries: Vec<DaMessageEntryV1>,
}

impl InboxBufferV1 {
    pub fn entries(&self) -> &[DaMessageEntryV1] {
        &self.entries
    }
}

impl LinearAccumulator for InboxBufferV1 {
    type InsertCnt = u16;
    type EntryData = DaMessageEntryV1;
    const MAX_INSERT: Self::InsertCnt = u16::MAX;

    fn insert(&mut self, entry: &Self::EntryData) {
        self.entries.push(entry.clone());
    }
}
