use borsh::{BorshDeserialize, to_vec};
use sled::IVec;
use ssz::{Decode, Encode};
use strata_db_types::traits::BlockStatus;
use strata_identifiers::OLBlockId;
use strata_ol_chain_types_new::OLBlock;
use typed_sled::codec::{CodecError, KeyCodec, ValueCodec};

use crate::{
    define_table_with_default_codec, define_table_with_integer_key, define_table_without_codec,
};

define_table_without_codec!(
    /// A table to store OL Block data. Maps block ID to Block
    (OLBlockSchema) OLBlockId => OLBlock
);

// OLBlockId uses default Borsh codec
impl KeyCodec<OLBlockSchema> for OLBlockId {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        to_vec(self).map_err(Into::into)
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        BorshDeserialize::deserialize_reader(&mut &data[..]).map_err(Into::into)
    }
}

define_table_with_default_codec!(
    /// A table to store OL Block status. Maps block ID to BlockStatus
    (OLBlockStatusSchema) OLBlockId => BlockStatus
);

define_table_with_integer_key!(
    /// A table to store OL Block IDs by slot. Maps slot to Vec<OLBlockId>
    (OLBlockHeightSchema) u64 => Vec<OLBlockId>
);

// OLBlock is SSZ-generated, so we use SSZ serialization instead of Borsh
impl ValueCodec<OLBlockSchema> for OLBlock {
    type Decoded = Self;

    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.as_ssz_bytes())
    }

    fn decode_value(data: IVec) -> Result<Self::Decoded, CodecError> {
        Self::from_ssz_bytes(data.as_ref()).map_err(|err| CodecError::DeserializationFailed {
            schema: OLBlockSchema::tree_name(),
            source: format!("SSZ decode error: {err:?}").into(),
        })
    }
}
