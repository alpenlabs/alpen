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

use arbitrary::Arbitrary;
use serde::{de, Deserialize, Deserializer, Serialize};
use serde_bytes::ByteBuf;
use strata_identifiers::Buf32;
use strata_l1_envelope_fmt::builder::MAX_ENVELOPE_PAYLOAD_SIZE;
use strata_l1_txfmt::TagData;

/// DA destination identifier. This will eventually be used to enable storing
/// payloads on alternative availability schemes.
///
/// Defined locally since `strata-btc-types` dropped its payload types in
/// v0.3.0; only the L1 settlement destination is currently supported.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum PayloadDest {
    /// If we expect the DA to be on the L1 chain that we settle to. This is
    /// always the strongest DA layer we have access to.
    L1 = 0,
}

/// Manual `Arbitrary` impl so that we always generate L1 DA if we add future
/// ones that would work in totally different ways.
impl<'a> Arbitrary<'a> for PayloadDest {
    fn arbitrary(_u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Self::L1)
    }
}

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
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct L1Payload {
    /// Wrapped so that serde encodes each chunk as a byte string rather than a sequence of
    /// integers; read it through [`L1Payload::data`].
    #[serde(rename = "payload")]
    data: Vec<ByteBuf>,

    #[serde(flatten)]
    tag: TagData,
}

impl<'de> Deserialize<'de> for L1Payload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Route reconstruction through `L1Payload::new` so that deserialized values are
        // validated against `MAX_ENVELOPE_PAYLOAD_SIZE` instead of trusting the input
        // verbatim. `TagData` validates itself the same way upstream.
        //
        // The helper is unavoidable: `#[serde(flatten)]` forces the buffered `Content` path,
        // so field attributes on the real struct cannot be reused here.
        #[derive(Deserialize)]
        struct Helper {
            #[serde(rename = "payload")]
            data: Vec<ByteBuf>,
            #[serde(flatten)]
            tag: TagData,
        }

        let helper = Helper::deserialize(deserializer)?;
        let data = helper.data.into_iter().map(ByteBuf::into_vec).collect();
        L1Payload::new(data, helper.tag).map_err(de::Error::custom)
    }
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
        let data = payload.into_iter().map(ByteBuf::from).collect();
        Ok(Self { data, tag })
    }

    /// Returns the data payload chunks.
    pub fn data(&self) -> impl ExactSizeIterator<Item = &[u8]> {
        self.data.iter().map(|chunk| chunk.as_slice())
    }

    /// Returns the tag metadata.
    pub fn tag(&self) -> &TagData {
        &self.tag
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

        Self::new(data, tag).map_err(|_| arbitrary::Error::IncorrectFormat)
    }
}

/// Intent produced when the sequencer wants to publish a payload to L1.
///
/// These are never stored on-chain.
// TODO(db-refactor-part-5): serde here is a stopgap so `IntentEntry` can be CBOR-encoded;
// the intent record wants restructuring into separate data and status parts.
#[derive(Clone, Debug, Eq, PartialEq, Arbitrary, Serialize, Deserialize)]
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

    /// Decoding must enforce the same size bound as [`L1Payload::new`]; a stored row is not
    /// trusted just because it is already in the database.
    #[test]
    fn deserialize_rejects_oversized_payload() {
        let oversized = serde_json::json!({
            "payload": [vec![0u8; MAX_ENVELOPE_PAYLOAD_SIZE + 1]],
            "subproto_id": 5,
            "tx_type": 9,
            "aux_data": [],
        });

        let err = serde_json::from_value::<L1Payload>(oversized)
            .expect_err("oversized payload must not decode");
        assert!(
            err.to_string().contains("exceeds maximum"),
            "unexpected error {err}"
        );
    }

    /// Chunks must reach CBOR as byte strings, not sequences of integers, which would cost
    /// roughly two bytes per payload byte.
    #[test]
    fn cbor_encodes_chunks_as_byte_strings() {
        let chunk_len = 1024;
        let payload = L1Payload::new(vec![vec![0xAB; chunk_len]], tag()).unwrap();

        let mut encoded = Vec::new();
        ciborium::into_writer(&payload, &mut encoded).unwrap();

        assert!(
            encoded.len() < chunk_len + 64,
            "expected a compact byte-string encoding, got {} bytes for {chunk_len} bytes of \
             payload",
            encoded.len()
        );

        let decoded: L1Payload = ciborium::from_reader(encoded.as_slice()).unwrap();
        assert_eq!(decoded, payload);
    }
}
