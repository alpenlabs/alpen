//! L1 data-availability payload types.
//!
//! [`L1Payload`] and [`PayloadIntent`] are defined here rather than sourced from
//! `strata-btc-types` because the upstream `L1Payload` caps each data chunk at
//! the 520-byte Bitcoin script-element size. That conflates a script push with a
//! logical envelope payload: a checkpoint is carried as a single logical payload
//! (the envelope builder splits it into 520-byte script pushes internally and the
//! ASM reader reassembles them), so a checkpoint for an epoch with account
//! activity — which exceeds 520 bytes — could never be posted, stalling
//! finalization. These local definitions bound the total payload by the envelope
//! limit instead.
// TODO(STR-3838): drop the upstream `L1Payload`/`PayloadIntent` once every
// consumer uses these local types.

use std::io::{self, Read, Write};

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
pub use strata_btc_types::payload::{BlobSpec, PayloadDest, PayloadSpec};
use strata_identifiers::Buf32;
use strata_l1_envelope_fmt::builder::MAX_ENVELOPE_PAYLOAD_SIZE;
use strata_l1_txfmt::TagData;

/// Error constructing an [`L1Payload`].
#[derive(Debug, thiserror::Error)]
pub enum L1PayloadError {
    /// The combined data chunks exceed [`MAX_ENVELOPE_PAYLOAD_SIZE`].
    #[error("payload of {total} bytes exceeds maximum of {MAX_ENVELOPE_PAYLOAD_SIZE}")]
    PayloadTooLarge {
        /// Combined length of all chunks.
        total: usize,
    },
}

/// Data that is submitted to L1. This can be DA, checkpoint, etc.
///
/// Each element of `data` is a logical payload that the envelope builder writes
/// as a single envelope, splitting it into Bitcoin script-element-sized pushes
/// internally. The total size is bounded by [`MAX_ENVELOPE_PAYLOAD_SIZE`].
///
/// The serde representation flattens the [`TagData`] fields alongside the
/// payload (`{payload, subproto_id, tx_type, aux_data}`).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct L1Payload {
    #[serde(rename = "payload")]
    data: Vec<Vec<u8>>,

    #[serde(flatten)]
    tag: TagData,
}

impl L1Payload {
    /// Creates a new L1 payload from data chunks and tag metadata.
    ///
    /// # Errors
    ///
    /// Returns [`L1PayloadError::PayloadTooLarge`] if the combined length of the
    /// chunks exceeds [`MAX_ENVELOPE_PAYLOAD_SIZE`].
    pub fn new(payload: Vec<Vec<u8>>, tag: TagData) -> Result<Self, L1PayloadError> {
        let total: usize = payload.iter().map(Vec::len).sum();
        if total > MAX_ENVELOPE_PAYLOAD_SIZE {
            return Err(L1PayloadError::PayloadTooLarge { total });
        }
        Ok(Self { data: payload, tag })
    }

    /// Returns the data payload chunks.
    pub fn data(&self) -> &[Vec<u8>] {
        &self.data
    }

    /// Returns the tag metadata.
    pub fn tag(&self) -> &TagData {
        &self.tag
    }
}

// Borsh is hand-rolled rather than derived for two reasons: `TagData` does not
// implement borsh, and the upstream `L1Payload` only gets borsh via
// `impl_borsh_via_ssz!`, which routes through the SSZ encoding that enforces the
// 520-byte per-chunk cap this type exists to avoid. So encode the payload chunks
// and the decomposed tag fields directly, routing decode through `TagData::new`
// and `L1Payload::new` to preserve their invariants.
impl BorshSerialize for L1Payload {
    fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        BorshSerialize::serialize(&self.data, writer)?;
        BorshSerialize::serialize(&self.tag.subproto_id(), writer)?;
        BorshSerialize::serialize(&self.tag.tx_type(), writer)?;
        BorshSerialize::serialize(&self.tag.aux_data().to_vec(), writer)?;
        Ok(())
    }
}

impl BorshDeserialize for L1Payload {
    fn deserialize_reader<R: Read>(reader: &mut R) -> io::Result<Self> {
        let data: Vec<Vec<u8>> = BorshDeserialize::deserialize_reader(reader)?;
        let subproto_id = u8::deserialize_reader(reader)?;
        let tx_type = u8::deserialize_reader(reader)?;
        let aux_data: Vec<u8> = BorshDeserialize::deserialize_reader(reader)?;
        let tag = TagData::new(subproto_id, tx_type, aux_data)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        Self::new(data, tag)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))
    }
}

impl<'a> Arbitrary<'a> for L1Payload {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        // Generate a bounded number of bounded chunks so the result is always a
        // valid payload.
        let num_chunks = u.int_in_range(0..=8)?;
        let mut data = Vec::with_capacity(num_chunks);
        for _ in 0..num_chunks {
            let chunk_len = u.int_in_range(0..=64)?;
            let mut chunk = Vec::with_capacity(chunk_len);
            for _ in 0..chunk_len {
                chunk.push(u8::arbitrary(u)?);
            }
            data.push(chunk);
        }

        let subproto_id = u8::arbitrary(u)?;
        let tx_type = u8::arbitrary(u)?;
        // `TagData` bounds aux data at 74 bytes.
        let aux_data_len = u.int_in_range(0..=74)?;
        let mut aux_data = Vec::with_capacity(aux_data_len);
        for _ in 0..aux_data_len {
            aux_data.push(u8::arbitrary(u)?);
        }

        let tag = TagData::new(subproto_id, tx_type, aux_data)
            .map_err(|_| arbitrary::Error::IncorrectFormat)?;

        Ok(Self { data, tag })
    }
}

/// Intent produced when the sequencer wants to publish a payload to L1.
///
/// These are never stored on-chain.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, BorshSerialize, BorshDeserialize)]
pub struct PayloadIntent {
    /// The destination for this payload.
    dest: PayloadDest,

    /// Commitment to the payload.
    commitment: Buf32,

    /// Blob payload.
    payload: L1Payload,
}

impl PayloadIntent {
    /// Creates a new payload intent with a destination, commitment, and payload.
    pub fn new(dest: PayloadDest, commitment: Buf32, payload: L1Payload) -> Self {
        Self {
            dest,
            commitment,
            payload,
        }
    }

    /// The target we expect the DA payload to be stored on.
    pub fn dest(&self) -> PayloadDest {
        self.dest
    }

    /// Commitment to the payload.
    pub fn commitment(&self) -> &Buf32 {
        &self.commitment
    }

    /// The payload that matches the commitment.
    pub fn payload(&self) -> &L1Payload {
        &self.payload
    }
}

#[cfg(test)]
mod tests {
    use super::{L1Payload, L1PayloadError, TagData, MAX_ENVELOPE_PAYLOAD_SIZE};

    fn tag() -> TagData {
        TagData::new(1, 1, vec![]).unwrap()
    }

    #[test]
    fn accepts_single_chunk_larger_than_script_element() {
        // A checkpoint for an active epoch is a single logical payload well over
        // the 520-byte Bitcoin script-element size; it must be accepted here and
        // chunked into script pushes by the envelope builder below this layer.
        let payload = vec![vec![0u8; 846]];
        assert!(L1Payload::new(payload, tag()).is_ok());
    }

    #[test]
    fn rejects_payload_over_total_max() {
        let payload = vec![vec![0u8; MAX_ENVELOPE_PAYLOAD_SIZE + 1]];
        assert!(matches!(
            L1Payload::new(payload, tag()),
            Err(L1PayloadError::PayloadTooLarge { .. })
        ));
    }

    #[test]
    fn borsh_roundtrip() {
        let payload = L1Payload::new(
            vec![vec![1, 2, 3], vec![4; 600]],
            TagData::new(5, 9, vec![0xAA, 0xBB]).unwrap(),
        )
        .unwrap();
        let buf = borsh::to_vec(&payload).unwrap();
        let decoded: L1Payload = borsh::from_slice(&buf).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn serde_flat_shape_roundtrip() {
        let payload = L1Payload::new(
            vec![vec![1, 2, 3]],
            TagData::new(5, 9, vec![0xAA, 0xBB]).unwrap(),
        )
        .unwrap();
        let value = serde_json::to_value(&payload).unwrap();
        let obj = value.as_object().unwrap();
        assert_eq!(obj["payload"], serde_json::json!([[1, 2, 3]]));
        assert_eq!(obj["subproto_id"], 5);
        assert_eq!(obj["tx_type"], 9);
        assert_eq!(obj["aux_data"], serde_json::json!([0xAA, 0xBB]));
        assert!(
            !obj.contains_key("tag"),
            "tag must be flattened, not nested"
        );

        let decoded: L1Payload = serde_json::from_value(value).unwrap();
        assert_eq!(decoded, payload);
    }
}
