use ssz::{Decode, Encode};
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_types::{OLAccountState, OLState, WriteBatch};
use typed_sled::codec::{CodecError, ValueCodec};

use crate::{define_table_without_codec, impl_codec_key_codec, impl_codec_value_codec};

// OLState is SSZ-generated, WriteBatch uses Codec
define_table_without_codec!(
    /// Table to store OLState snapshots keyed by OLBlockCommitment.
    (OLStateSchema) OLBlockCommitment => OLState
);

define_table_without_codec!(
    /// Table to store OL state write batches keyed by OLBlockCommitment.
    (OLWriteBatchSchema) OLBlockCommitment => WriteBatch<OLAccountState>
);

// OLBlockCommitment uses Codec for key encoding (big-endian for proper linear scans)
impl_codec_key_codec!(OLStateSchema, OLBlockCommitment);
impl_codec_key_codec!(OLWriteBatchSchema, OLBlockCommitment);

// OLState is SSZ-generated, use SSZ serialization directly
impl ValueCodec<OLStateSchema> for OLState {
    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: &[u8]) -> Result<Self, CodecError> {
        Self::from_ssz_bytes(data).map_err(|err| CodecError::SerializationFailed {
            schema: OLStateSchema::tree_name(),
            source: format!("SSZ decode error: {err:?}").into(),
        })
    }
}

// WriteBatch uses Codec trait (contains non-SSZ types like BTreeMap, SerialMap)
impl_codec_value_codec!(OLWriteBatchSchema, WriteBatch<OLAccountState>);
