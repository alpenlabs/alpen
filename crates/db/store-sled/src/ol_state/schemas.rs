use sled::IVec;
use ssz::{Decode, Encode};
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types_v1::{OLStateV1, WriteBatch};
use typed_sled::codec::{CodecError, ValueCodec};

use crate::{define_table_without_codec, impl_codec_key_codec, impl_codec_value_codec};

// OLStateV1 is SSZ-generated, WriteBatch uses Codec
define_table_without_codec!(
    /// Table to store OLStateV1 snapshots keyed by OLBlockCommitment.
    (OLStateSchema) OLBlockCommitment => OLStateV1
);

define_table_without_codec!(
    /// Table to store OL state write batches keyed by OLBlockCommitment.
    (OLWriteBatchSchema) OLBlockCommitment => WriteBatch
);

// OLBlockCommitment uses Codec for key encoding (big-endian for proper linear scans)
impl_codec_key_codec!(OLStateSchema, OLBlockCommitment);
impl_codec_key_codec!(OLWriteBatchSchema, OLBlockCommitment);

// OLStateV1 is SSZ-generated, use SSZ serialization directly
impl ValueCodec<OLStateSchema> for OLStateV1 {
    type Decoded = Self;

    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: IVec) -> Result<Self::Decoded, CodecError> {
        Self::from_ssz_bytes(data.as_ref()).map_err(|err| CodecError::DeserializationFailed {
            schema: OLStateSchema::tree_name(),
            source: format!("SSZ decode error: {err:?}").into(),
        })
    }
}

// WriteBatch uses Codec trait (contains non-SSZ types like BTreeMap, SerialMap)
impl_codec_value_codec!(OLWriteBatchSchema, WriteBatch);
